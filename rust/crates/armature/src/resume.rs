//! 閉じたタブを戻す(⌘⇧T)ときに渡す model と effort を、会話ログから読む。
//!
//! `claude --resume <id>` は会話を戻すだけで、model と effort は**その時点の既定**で立つ。
//! しかも既定は動く——`/model` も `/effort` も「saved as your default for new sessions」で、
//! 打ったタブの外まで既定を書き換える。別のタブで打ち替えた値が、戻したタブへ黙って
//! 乗っていた。
//!
//! どちらも**会話ログのいちばん新しい値**が今の姿:
//!
//! - model: 窓が立つたび(と打ち替えた次の手番)に書かれる `"attachment":{"type":"model"}` の
//!   `modelId`。`claude-opus-5-5[1m]` のように**窓の広さまで入った正式名**で、族の名だけの
//!   `opus` では 200k と 1M を分けられない。それより後に `/model` を打っていれば、その控え
//! - effort: AI の発言の行の `effort`。それより後に打った `/effort`・`/model … with `max` effort`
//!
//! 読めなかった欄は渡さない(既定に任せる)——**当て推量の値で起こさない**。

use memchr::memmem;

/// effort として渡してよい語。
const LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// 字で当たった行を JSON に起こして確かめる数の蓋。
///
/// **字だけで当てない**——道具の戻りに同じ並びが混ざる(会話ログやこのファイルを
/// 読んだ回。2026-09-24 に実物で当たった)。後ろから順に確かめ、形の合わない行は飛ばす。
const TRIES: usize = 32;

/// 窓が立ったときのモデル。
const ATTACHED: &[u8] = br#""attachment":{"type":"model""#;
/// `/model` の控え。
const SET_MODEL: &[u8] = b"<local-command-stdout>Set model to ";
/// `/effort` の控え。
const SET_EFFORT: &[u8] = b"<local-command-stdout>Set effort level to ";
/// AI の発言の行。
const SPOKEN: &[u8] = br#""type":"assistant""#;

/// 戻すときに渡す値。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resumed {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Resumed {
    /// `claude` へ足す引数。読めなかった欄は渡さない。
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(model) = &self.model {
            args.extend(["--model".to_string(), model.clone()]);
        }
        if let Some(effort) = &self.effort {
            args.extend(["--effort".to_string(), effort.clone()]);
        }
        args
    }
}

/// その会話を戻すときの値。会話ログが見つからなければ両方 `None`。
///
/// ログは丸ごと読む(大きいものは 80MB 級)。正式名は窓が立った刻にしか書かれず、
/// 長い会話では頭のほうにしか無い。押すのは事故のあとの1回だけなので、主線で読んでも
/// 数十 ms で済む。
#[must_use]
pub fn for_session(session_id: &str) -> Resumed {
    let Some(path) = armature_core::monitor::transcript_path(session_id) else {
        return Resumed::default();
    };
    std::fs::read(path)
        .map(|log| read(&log))
        .unwrap_or_default()
}

/// 会話ログ1本から読む(ファイルを触らない側・試験用の入口)。
#[must_use]
pub fn read(log: &[u8]) -> Resumed {
    let attached = last_line(log, ATTACHED, attached_model);
    let declared = last_line(log, SET_MODEL, declared_model);
    let effort_set = last_line(log, SET_EFFORT, declared_effort);
    let spoken = last_line(log, SPOKEN, spoken);

    // model: 正式名と `/model` の控えのうち、新しいほう。
    let mut model = attached;
    if let Some((at, Declared { model: Some(alias), .. })) = &declared
        && model.as_ref().is_none_or(|(before, _)| at > before)
    {
        model = Some((*at, alias.clone()));
    }
    // 最後に喋った AI の族と食い違うなら、喋った側を信じる——控えの書式が
    // 変わって読めなくなった回の保険。族が合っていれば正式名のほうが細かい。
    if let Some((at, Spoken { family, .. })) = &spoken {
        let agrees = model
            .as_ref()
            .is_some_and(|(_, id)| armature_core::monitor::model_family(id).as_deref() == Some(family));
        if !agrees && model.as_ref().is_none_or(|(before, _)| at > before) {
            model = Some((*at, alias_of(family)));
        }
    }

    // effort: 発言の行・`/effort`・`/model` の控えのうち、いちばん新しいもの。
    let mut effort = spoken.and_then(|(at, said)| said.effort.map(|effort| (at, effort)));
    let typed = [
        effort_set,
        declared.and_then(|(at, set)| set.effort.map(|effort| (at, effort))),
    ];
    for (at, level) in typed.into_iter().flatten() {
        if effort.as_ref().is_none_or(|(before, _)| at > *before) {
            effort = Some((at, level));
        }
    }

    Resumed {
        model: model.map(|(_, model)| model),
        effort: effort.map(|(_, effort)| effort),
    }
}

/// `needle` を含む行を後ろから見て、`read` が値を返した最初の1行(=いちばん新しい行)の
/// 行頭の位置と値。サブエージェントの行(`"isSidechain":true`)は親の姿ではないので飛ばす。
fn last_line<T>(
    log: &[u8],
    needle: &[u8],
    read: impl Fn(&serde_json::Value) -> Option<T>,
) -> Option<(usize, T)> {
    let mut end = log.len();
    let mut tries = 0;
    while tries < TRIES {
        let hit = memmem::rfind(&log[..end], needle)?;
        let start = memchr::memrchr(b'\n', &log[..hit]).map_or(0, |at| at + 1);
        let stop = memchr::memchr(b'\n', &log[hit..]).map_or(log.len(), |at| hit + at);
        let line = &log[start..stop];
        end = start;
        // 傍流の印は行の頭の数十字に出る。JSON に起こす前に字で落とす(数に入れない)。
        if memmem::find(&line[..line.len().min(256)], br#""isSidechain":true"#).is_some() {
            continue;
        }
        tries += 1;
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if value["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        if let Some(found) = read(&value) {
            return Some((start, found));
        }
    }
    None
}

/// 窓が立ったときの正式名。
fn attached_model(line: &serde_json::Value) -> Option<String> {
    let attachment = &line["attachment"];
    if attachment["type"].as_str() != Some("model") {
        return None;
    }
    let id = attachment["identity"]["modelId"].as_str()?.trim();
    plain_model_id(id).then(|| id.to_string())
}

/// `/model` の控えから読めたもの。
struct Declared {
    model: Option<String>,
    effort: Option<String>,
}

/// 手で打ったスラッシュコマンドの控えの本文。**人の行で、本文が字そのもの**のときだけ
/// ——道具の戻り(本文が配列)に同じ字が入っている回を落とす。
fn command_output<'a>(line: &'a serde_json::Value, head: &str) -> Option<&'a str> {
    if line["type"].as_str() != Some("user") {
        return None;
    }
    let body = line["message"]["content"]
        .as_str()?
        .strip_prefix("<local-command-stdout>")?
        .strip_prefix(head)?;
    Some(body.strip_suffix("</local-command-stdout>").unwrap_or(body))
}

/// `Set model to `Opus 5.5 (1M context)` and saved as your default for new sessions with `max` effort`。
/// 昔の版は名前を backtick ではなく太字の制御文字で囲む(`\u001b[1mFable 5\u001b[22m`)。
fn declared_model(line: &serde_json::Value) -> Option<Declared> {
    let body = command_output(line, "Set model to ")?;
    let (name, rest) = body.split_once(" and saved").unwrap_or((body, ""));
    let model = armature_core::monitor::model_family(name).map(|family| {
        if name.contains("1M") {
            format!("{family}[1m]")
        } else {
            family
        }
    });
    let effort = rest
        .split_once("with `")
        .and_then(|(_, tail)| tail.split_once('`'))
        .map(|(word, _)| word)
        .filter(|word| LEVELS.contains(word))
        .map(str::to_string);
    (model.is_some() || effort.is_some()).then_some(Declared { model, effort })
}

/// `Set effort level to max (this session only): …`。
fn declared_effort(line: &serde_json::Value) -> Option<String> {
    let body = command_output(line, "Set effort level to ")?;
    let word: String = body.chars().take_while(char::is_ascii_alphanumeric).collect();
    LEVELS.contains(&word.as_str()).then_some(word)
}

/// AI の発言の行から読めたもの。
struct Spoken {
    family: String,
    effort: Option<String>,
}

/// 族の名が読めない発言(`<synthetic>` の失敗の控え等)は AI の手番に数えない。
fn spoken(line: &serde_json::Value) -> Option<Spoken> {
    if line["type"].as_str() != Some("assistant") {
        return None;
    }
    let family = armature_core::monitor::model_family(line["message"]["model"].as_str()?)?;
    let effort = line["effort"]
        .as_str()
        .filter(|effort| LEVELS.contains(effort))
        .map(str::to_string);
    Some(Spoken { family, effort })
}

/// 族の名しか分からないときの呼び名。**Opus は 1M 版**——器が起こす Opus は全部
/// `opus[1m]` で(`terminal.rs` の `DEFAULT_CLAUDE_MODEL`・`task_arguments`)、素の
/// `opus` を渡すと 200k で立つ。
fn alias_of(family: &str) -> String {
    if family == "opus" {
        "opus[1m]".to_string()
    } else {
        family.to_string()
    }
}

/// `claude --model` へそのまま渡せる綴りか。英数と `-` `.` `_`、末尾に `[1m]` の類を1つまで。
fn plain_model_id(id: &str) -> bool {
    let (name, window) = match id.split_once('[') {
        Some((name, window)) => (name, Some(window)),
        None => (id, None),
    };
    let word = |text: &str| {
        !text.is_empty()
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
    };
    word(name)
        && window.is_none_or(|window| {
            window
                .strip_suffix(']')
                .is_some_and(|inner| !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_alphanumeric()))
        })
}

#[cfg(test)]
mod tests {
    use super::{Resumed, read};

    fn attached(id: &str) -> String {
        format!(
            r#"{{"parentUuid":null,"isSidechain":false,"attachment":{{"type":"model","identity":{{"modelId":"{id}","marketingName":"x"}}}},"type":"attachment"}}"#
        )
    }

    fn said(model: &str, effort: &str) -> String {
        format!(
            r#"{{"parentUuid":"a","isSidechain":false,"message":{{"model":"{model}","type":"message","role":"assistant","content":[{{"type":"text","text":"はい"}}]}},"type":"assistant","effort":"{effort}","perTurnEffort":null}}"#
        )
    }

    fn typed(stdout: &str) -> String {
        format!(
            r#"{{"parentUuid":"a","isSidechain":false,"type":"user","message":{{"role":"user","content":"<local-command-stdout>{stdout}</local-command-stdout>"}}}}"#
        )
    }

    fn log(lines: &[String]) -> Vec<u8> {
        let mut text = lines.join("\n");
        text.push('\n');
        text.into_bytes()
    }

    /// 窓が立ったときの正式名(1M の印まで)と、最後の発言の effort で戻すこと。
    #[test]
    fn the_attached_model_id_and_the_last_effort_win() {
        let got = read(&log(&[
            attached("claude-opus-5-5[1m]"),
            said("claude-opus-5-5", "high"),
            said("claude-opus-5-5", "max"),
        ]));
        assert_eq!(
            got,
            Resumed {
                model: Some("claude-opus-5-5[1m]".into()),
                effort: Some("max".into()),
            }
        );
        assert_eq!(
            got.arguments(),
            ["--model", "claude-opus-5-5[1m]", "--effort", "max"]
        );
    }

    /// 最後の手番の後に `/model` を打っていたら、打った値で戻すこと(effort も控えから)。
    #[test]
    fn a_model_typed_after_the_last_turn_is_kept() {
        let got = read(&log(&[
            attached("claude-opus-5-5[1m]"),
            said("claude-opus-5-5", "max"),
            typed("Set model to `Fable 5.1` and saved as your default for new sessions with `high` effort"),
        ]));
        assert_eq!(got.model.as_deref(), Some("fable"));
        assert_eq!(got.effort.as_deref(), Some("high"));

        let wide = read(&log(&[
            attached("claude-fable-5-1"),
            typed("Set model to `Opus 5.5 (1M context)` and saved as your default for new sessions"),
        ]));
        assert_eq!(wide.model.as_deref(), Some("opus[1m]"));
    }

    /// `/model` の後に手番があれば、その手番で書かれた正式名のほうが細かい。
    #[test]
    fn a_later_attachment_beats_the_typed_alias() {
        let got = read(&log(&[
            attached("claude-opus-5-5[1m]"),
            typed("Set model to `Fable 5.1` and saved as your default for new sessions"),
            attached("claude-fable-5-1"),
            said("claude-fable-5-1", "xhigh"),
        ]));
        assert_eq!(got.model.as_deref(), Some("claude-fable-5-1"));
        assert_eq!(got.effort.as_deref(), Some("xhigh"));
    }

    /// 最後の手番の後に `/effort` を打っていたら、その値。
    #[test]
    fn an_effort_typed_after_the_last_turn_is_kept() {
        let got = read(&log(&[
            attached("claude-opus-5-5[1m]"),
            said("claude-opus-5-5", "high"),
            typed("Set effort level to max (this session only): Maximum capability with deepest reasoning."),
        ]));
        assert_eq!(got.effort.as_deref(), Some("max"));
    }

    /// 道具の戻りに同じ字が入っていても拾わないこと(会話ログやこのファイルを読んだ回)。
    #[test]
    fn tool_results_quoting_the_same_words_are_ignored() {
        let quoted = r#"{"parentUuid":"a","isSidechain":false,"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"<local-command-stdout>Set model to `Haiku 4.5` and saved with `low` effort</local-command-stdout> <local-command-stdout>Set effort level to low"}]}}"#;
        let got = read(&log(&[
            attached("claude-opus-5-5[1m]"),
            said("claude-opus-5-5", "max"),
            quoted.to_string(),
        ]));
        assert_eq!(got.model.as_deref(), Some("claude-opus-5-5[1m]"));
        assert_eq!(got.effort.as_deref(), Some("max"));
    }

    /// サブエージェントの行は親の姿ではない。
    #[test]
    fn sidechain_lines_are_not_the_parent() {
        let side = said("claude-sonnet-5", "low").replace(r#""isSidechain":false"#, r#""isSidechain":true"#);
        let got = read(&log(&[
            attached("claude-opus-5-5[1m]"),
            said("claude-opus-5-5", "xhigh"),
            side,
        ]));
        assert_eq!(got.model.as_deref(), Some("claude-opus-5-5[1m]"));
        assert_eq!(got.effort.as_deref(), Some("xhigh"));
    }

    /// 正式名が無い古いログでも、発言の族から戻す(Opus は 1M 版)。
    #[test]
    fn the_spoken_family_is_the_fallback() {
        let got = read(&log(&[said("claude-opus-5", "high")]));
        assert_eq!(got.model.as_deref(), Some("opus[1m]"));
        let fable = read(&log(&[attached("claude-opus-5-5[1m]"), said("claude-fable-5-1", "high")]));
        assert_eq!(fable.model.as_deref(), Some("fable"));
    }

    /// 実物の会話ログで読めるか。`COCKPIT_RESUME_PROBE=1` のときだけ走る。
    /// 台帳(`cockpit-sessions.json`)の閉じた控えを新しい順に 8 件、⌘⇧T が渡す引数と
    /// 読むのに掛かった時間を出す。
    #[test]
    fn 実物の会話ログ() {
        if std::env::var_os("COCKPIT_RESUME_PROBE").is_none() {
            return;
        }
        let path = armature_core::geo::state_dir().join("cockpit-sessions.json");
        let text = std::fs::read_to_string(path).expect("台帳が読める");
        let rows: Vec<serde_json::Value> = serde_json::from_str(&text).expect("台帳が JSON");
        let mut closed: Vec<(u64, String, String)> = rows
            .iter()
            .filter_map(|row| {
                Some((
                    row["closed_at"].as_u64()?,
                    row["id"].as_str()?.to_string(),
                    row["tab"].as_str().unwrap_or_default().to_string(),
                ))
            })
            .collect();
        closed.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, id, tab) in closed.into_iter().take(8) {
            let started = std::time::Instant::now();
            let got = super::for_session(&id);
            println!(
                "{id} {tab}: {:?} ({} ms)",
                got.arguments(),
                started.elapsed().as_millis()
            );
        }
    }

    /// 何も読めなければ何も渡さない(既定に任せる)。綴りの怪しい正式名も渡さない。
    #[test]
    fn nothing_readable_passes_nothing() {
        assert_eq!(read(b"").arguments(), Vec::<String>::new());
        let odd = read(&log(&[attached("opus; rm -rf ~")]));
        assert_eq!(odd.model, None);
        let synthetic = read(&log(&[said("<synthetic>", "high")]));
        assert_eq!(synthetic, Resumed::default());
    }
}
