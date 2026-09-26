//! 子プロセスの起動口。**生 `Command::spawn` は使わない**——必ずここを通す。
//!
//! 理由: `spawn` しっぱなしの子は終了してもゾンビとして残り、`Child` を
//! drop しても Rust は wait しない(std の仕様)。盤はセッション起動・
//! 外部ビューアの呼び出しでプロセスを撒くので、放っておくと常駐の TUI に
//! ゾンビが積もる。回収スレッドを1本添えるだけで済む話なので、口を1つに
//! 絞って必ず添える。

use std::io;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::thread;

/// 子へ引き継いではいけない、親のセッションの印。
///
/// 窓をどこから起こしたかで中身が変わる——Claude Code のセッションから `open` で
/// 起こすと `CLAUDE_CODE_CHILD_SESSION` 等が丸ごと相続され、**タブの中で起きた
/// Claude Code が自分を「子セッション」と誤認して transcript の保存を切る**。
/// 保存が切れると会話ログが残らず、セッション一覧のモデル・effort も出どころを
/// 失って空欄のままになる。窓の中で起きるものは新しい1本であって、窓を起こした
/// 誰かの続きではない。
///
/// 落とすのは前置き `CLAUDE_CODE_` の全部と、ここに並べた名指しの数個。
/// `ANTHROPIC_*` や `CLAUDE_CONFIG_DIR` のような**設定**には触らない
/// ——あれは利用者の環境の一部で、子も同じものを読んでよい。
const INHERITED_MARKERS: [&str; 4] = ["CLAUDECODE", "CLAUDE_PID", "CLAUDE_EFFORT", "AI_AGENT"];

/// locale が無いまま起きたときに使う既定。値そのものより UTF-8 であることが要る。
pub const DEFAULT_LOCALE: &str = "en_US.UTF-8";

/// 落とす対象か。子の env と tmux サーバの env を同じ物差しで測る。
#[must_use]
pub fn is_inherited_marker(key: &str) -> bool {
    key.starts_with("CLAUDE_CODE_") || INHERITED_MARKERS.contains(&key)
}

/// 子プロセスに渡す環境の差分。
///
/// **窓自身の環境は起動したときのまま触らない。**`set_var` / `remove_var` は
/// スレッドが1本でも立っていると未定義動作になる。Armature はライブラリで、外の
/// crate の `main` やパネルが先にスレッドを立てうるので、「まだスレッドが無い」
/// 前提は置けない。代わりに子を起こすたびに [`child_env`] でこの差分を当てる。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChildEnv {
    /// 子から落とす名前。
    pub remove: Vec<String>,
    /// 子に据える名前と値。
    pub set: Vec<(String, String)>,
}

impl ChildEnv {
    /// 窓の環境(`vars`)から差分を組む。
    ///
    /// - 親のセッションの印([`is_inherited_marker`])と `NO_COLOR` は落とす
    /// - `PATH` の前に CLI の棚を足す。Dock から起こした窓はログインシェルの PATH を
    ///   持たないので、足さないと CLI から起こしたときと別の実行ファイルを見る
    /// - locale が無ければ UTF-8 を据える。tmux は非 UTF-8 と判定すると受け取った
    ///   マルチバイト文字を `_` へ潰す(罫線もかな漢字も消える)。CLI 起動では親の
    ///   シェルから渡るので、Dock・launchd から起こしたときにだけ要る
    /// - 端末の色は truecolor で出させる(`COLORTERM`・`FORCE_COLOR`)
    #[must_use]
    pub fn from_vars(vars: &[(String, String)]) -> Self {
        let get = |name: &str| {
            vars.iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let mut remove: Vec<String> = vars
            .iter()
            .map(|(key, _)| key)
            .filter(|key| is_inherited_marker(key))
            .cloned()
            .collect();
        remove.push("NO_COLOR".to_string());
        let mut set = vec![(
            "PATH".to_string(),
            runtime_path(get("HOME").unwrap_or_default(), get("PATH").unwrap_or_default()),
        )];
        if !locale_is_configured([get("LC_ALL"), get("LC_CTYPE"), get("LANG")]) {
            set.push(("LANG".to_string(), DEFAULT_LOCALE.to_string()));
        }
        set.push(("COLORTERM".to_string(), "truecolor".to_string()));
        set.push(("FORCE_COLOR".to_string(), "3".to_string()));
        Self { remove, set }
    }

    /// 差分を `command` に当てる。
    pub fn apply<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        for key in &self.remove {
            command.env_remove(key);
        }
        for (key, value) in &self.set {
            command.env(key, value);
        }
        command
    }
}

/// この窓の子に渡す環境の差分。最初に要ったときに窓の環境から1度だけ組む(読むだけ)。
#[must_use]
pub fn child_environment() -> &'static ChildEnv {
    static ENV: OnceLock<ChildEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let vars: Vec<(String, String)> = std::env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        ChildEnv::from_vars(&vars)
    })
}

/// 子を起こす前に必ず通す。窓の環境から親のセッションの印を落とし、PATH・locale・色を据える。
pub fn child_env(command: &mut Command) -> &mut Command {
    child_environment().apply(command)
}

/// `PATH` の前に CLI の棚(`~/.local/bin`・`~/.cargo/bin`・Homebrew)を足す。
#[must_use]
pub fn runtime_path(home: &str, current: &str) -> String {
    let mut entries = Vec::new();
    if !home.is_empty() {
        entries.push(format!("{home}/.local/bin"));
        entries.push(format!("{home}/.cargo/bin"));
    }
    entries.extend(["/opt/homebrew/bin".into(), "/usr/local/bin".into()]);
    entries.extend(
        current
            .split(':')
            .filter(|entry| !entry.is_empty())
            .map(ToString::to_string),
    );
    entries.dedup();
    entries.join(":")
}

fn locale_is_configured<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> bool {
    values.into_iter().flatten().any(|value| !value.is_empty())
}

/// 子を起動し、終了を待つ回収スレッドを添える。返すのは pid。
///
/// 「起動したら投げっぱなし」の用途(ビューアを開く・セッションを立てる)
/// 専用。出力を読みたい・終了コードが要る場合はここを使わず、呼び手が
/// `wait_with_output()` まで面倒を見ること。
pub fn spawn_reaped(mut cmd: Command) -> io::Result<u32> {
    let child = cmd.spawn()?;
    Ok(reap(child))
}

/// 標準入出力を捨てて起動する版。TUIの画面へ子の出力が混ざるのを防ぐ
/// ——raw モードの alternate screen に他人の stdout が刺さると表示が壊れる。
pub fn spawn_reaped_quiet(mut cmd: Command) -> io::Result<u32> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    spawn_reaped(cmd)
}

/// 既に起動済みの子に回収スレッドを付ける。
fn reap(mut child: Child) -> u32 {
    let pid = child.id();
    let _ = thread::Builder::new()
        .name(format!("board-reap-{pid}"))
        .spawn(move || {
            let _ = child.wait();
        });
    pid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn the_parent_session_markers_do_not_reach_the_children() {
        // 相続すると、タブの中の Claude Code が自分を子セッションと誤認して
        // transcript の保存を切り、モデルと effort の出どころまで消える。
        let env = ChildEnv::from_vars(&vars(&[
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_SESSION_ID", "abc"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CONFIG_DIR", "/tmp/claude"),
            ("ANTHROPIC_MODEL", "x"),
        ]));
        for gone in ["CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_SESSION_ID", "CLAUDECODE", "NO_COLOR"] {
            assert!(env.remove.iter().any(|key| key == gone), "{gone} が残る");
        }
        // 設定は利用者の環境の一部。落としてはいけない側。
        assert!(!env.remove.iter().any(|key| key == "CLAUDE_CONFIG_DIR"));
        assert!(!env.remove.iter().any(|key| key == "ANTHROPIC_MODEL"));
    }

    #[test]
    fn the_child_env_is_applied_to_the_command_not_to_the_app() {
        let env = ChildEnv::from_vars(&vars(&[
            ("HOME", "/Users/test"),
            ("PATH", "/usr/bin:/bin"),
            ("CLAUDECODE", "1"),
        ]));
        let mut command = Command::new("/usr/bin/env");
        env.apply(&mut command);
        let changes: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(changes.contains(&("CLAUDECODE".to_string(), None)));
        assert!(changes.contains(&("NO_COLOR".to_string(), None)));
        assert!(changes.contains(&(
            "PATH".to_string(),
            Some(
                "/Users/test/.local/bin:/Users/test/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"
                    .to_string()
            )
        )));
        assert!(changes.contains(&("LANG".to_string(), Some(DEFAULT_LOCALE.to_string()))));
        assert!(changes.contains(&("COLORTERM".to_string(), Some("truecolor".to_string()))));
        assert!(changes.contains(&("FORCE_COLOR".to_string(), Some("3".to_string()))));
        // 子を実際に起こしても印は届かない。
        let output = command.output().expect("env が走る");
        let seen = String::from_utf8_lossy(&output.stdout);
        assert!(!seen.lines().any(|line| line.starts_with("CLAUDECODE=")));
        assert!(seen.lines().any(|line| line == "FORCE_COLOR=3"));
    }

    #[test]
    fn a_configured_locale_is_left_alone() {
        let set = |pairs: &[(&str, &str)]| ChildEnv::from_vars(&vars(pairs)).set;
        assert!(!set(&[("LANG", "ja_JP.UTF-8")]).iter().any(|(key, _)| key == "LANG"));
        assert!(!set(&[("LC_ALL", "C.UTF-8")]).iter().any(|(key, _)| key == "LANG"));
        // 空の値は据えていないのと同じ。
        assert!(set(&[("LANG", ""), ("LC_CTYPE", "")]).iter().any(|(key, _)| key == "LANG"));
        assert!(DEFAULT_LOCALE.contains("UTF-8"));
    }

    #[test]
    fn dock_path_keeps_cli_locations_and_existing_system_entries() {
        assert_eq!(
            runtime_path("/Users/test", "/usr/bin:/bin"),
            "/Users/test/.local/bin:/Users/test/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"
        );
    }

    #[test]
    fn spawns_and_reaps() {
        let mut cmd = Command::new("/usr/bin/true");
        cmd.stdout(Stdio::null());
        let pid = spawn_reaped(cmd).expect("起動できる");
        assert!(pid > 0);
        // 回収スレッドが wait を持っているので、ここで待たなくてもゾンビは残らない。
        // (残るかどうかの確認は ps 依存になるため、ここでは起動できたことまで)
    }
}
