//! 状態の置き場の `config.env` から機体ごとの設定を読む(`TASKS_HOME`・`CLAUDE_SESSION_CWD`)。
//!
//! 解決順は「このファイル → 既定」。**機体差はこのファイルに置く**——束から起こした
//! 窓には環境変数が1つも届かないので、環境変数は読まない。

/// `config.env` の値を読む。
pub fn read_value(name: &str) -> Option<String> {
    let path = armature_core::paths::state().join("config.env");
    let text = std::fs::read_to_string(path).ok()?;
    value_from(&text, name)
}

/// 同じキーが複数あれば後ろの行が勝つ(shell が source したときと同じ向き)。
fn value_from(text: &str, name: &str) -> Option<String> {
    text.lines()
        .rev()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            (key.trim() == name).then(|| value.trim().trim_matches(['"', '\'']).to_string())
        })
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::value_from;

    #[test]
    fn reads_the_key() {
        assert_eq!(
            value_from("TASKS_HOME=/Users/x/Tasks\n", "TASKS_HOME").as_deref(),
            Some("/Users/x/Tasks")
        );
    }

    #[test]
    fn ignores_comments_and_other_keys() {
        let text = "# TASKS_HOME=/commented\nOTHER=1\nTASKS_HOME=/real\n";
        assert_eq!(value_from(text, "TASKS_HOME").as_deref(), Some("/real"));
    }

    #[test]
    fn strips_export_and_quotes() {
        assert_eq!(
            value_from("export CLAUDE_SESSION_CWD=\"~/dev\"\n", "CLAUDE_SESSION_CWD").as_deref(),
            Some("~/dev")
        );
    }

    #[test]
    fn the_last_line_wins() {
        let text = "CLAUDE_SESSION_CWD=~/work\nCLAUDE_SESSION_CWD=~/dev\n";
        assert_eq!(
            value_from(text, "CLAUDE_SESSION_CWD").as_deref(),
            Some("~/dev")
        );
    }

    #[test]
    fn an_empty_value_is_not_a_setting() {
        let text = "CLAUDE_SESSION_CWD=~/dev\nCLAUDE_SESSION_CWD=\n";
        assert_eq!(
            value_from(text, "CLAUDE_SESSION_CWD").as_deref(),
            Some("~/dev")
        );
    }
}
