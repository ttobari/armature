//! 頁の本文を日本語にするアプリ内ブラウザ向けの共有部。
//! 。
//! 収集と差し替えは
//! 頁へ仕込んだJS、判定と自動翻訳はアプリ束のApple Translation、`T`の精訳だけ
//! claude CLIが担う。
//!
//! Appleの`Translation`はSwift限定APIなので、この共有crateは訳文の型・日本語判定と
//! Claude精訳だけを持つ。外部のネットワーク翻訳サービスへは接続しない。

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 精訳(claude CLI)の待ち時間の上限。
///
/// 返ってこないまま居座らせない——頁のバッジが「精訳中…」で固まる。
pub const CLAUDE_TIMEOUT: Duration = Duration::from_secs(180);

/// 精訳 1 回に渡す本文の上限。頁の JS は 20 塊ずつ送る。
const CLAUDE_MAX_TEXTS: usize = 20;
const CLAUDE_MAX_BYTES: usize = 256 * 1024;

/// 精訳は同時に 1 本。後から来た頼みは前が終わるまで待つ。
static CLAUDE_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 訳した結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Translated {
    /// 訳文。**入力と同じ長さ・同じ順序**。
    pub texts: Vec<String>,
    /// 頁の元の言語。分からなければ空。
    ///
    /// 日本語の頁を訳し直して壊すのがいちばんまずいので、呼び手はこれで弾く
    /// ([`is_japanese`])。
    pub lang: String,
}

/// 日本語の頁か。**訳しにいってはいけない側**。
///
/// はてブ経由で開いた日本語記事を訳し直すと、原文が壊れたまま戻せなくなる。
#[must_use]
pub fn is_japanese(lang: &str) -> bool {
    lang == "ja" || lang.starts_with("ja-")
}

/// 訳す先(いまの表示の言語)と同じ言語の頁か。**訳しにいってはいけない側**。
#[must_use]
pub fn is_target_language(lang: &str) -> bool {
    let target = crate::lang::current().key();
    lang == target || lang.starts_with(&format!("{target}-"))
}

/// 自動翻訳はアプリ束のApple Translationから呼ぶ。
///
/// 互換用に入口は残すが、この共有crateからネットワーク翻訳へは接続しない。
///
/// # Errors
/// 渡すものが無いとき、またはアプリ側の翻訳器を使う必要があるとき。
pub fn translate(texts: &[String]) -> Result<Translated, String> {
    if texts.is_empty() {
        return Err(crate::tr!("Nothing to translate", "訳す本文が無い").to_string());
    }
    Err("automatic translation needs the app's Apple Translation".to_string())
}

/// 精訳の CLI(利用者の Claude Code)。決まった棚を見て、無ければログインシェルに聞く。
fn claude_bin() -> std::path::PathBuf {
    use std::path::PathBuf;
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    for path in [
        home.join(".local/bin/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
        home.join(".claude/local/claude"),
    ] {
        if path.is_file() {
            return path;
        }
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    crate::proc::child_env(&mut std::process::Command::new(shell))
        .args(["-l", "-c", "command -v claude"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.lines().last().map(|line| PathBuf::from(line.trim())))
        .unwrap_or_else(|| home.join(".local/bin/claude"))
}

/// claudeへ渡す文面。**同じ長さのJSON配列だけ**を返させる。訳す先はいまの表示の言語。
#[must_use]
pub fn claude_prompt(json: &str) -> String {
    crate::tr!(
        format!(
            "The JSON array below holds blocks of text taken from a web page. Translate each element \
             into natural English and output only a JSON array of the same length and order. No \
             explanations, preambles or code fences. Put elements that need no translation (proper \
             nouns only, for example) in as they are.\n\n\
             {json}"
        ),
        format!(
            "次のJSON配列はWebページから抜き出した本文ブロックです。各要素を自然な日本語に\
             訳し、同じ長さ・同じ順序のJSON配列だけを出力してください。説明・前置き・コードフェンスは\
             一切付けないこと。訳す必要がない要素(固有名詞だけ等)は原文のまま入れてください。\n\n\
             {json}"
        ),
    )
}

/// claudeの返事から配列だけを取り出す。
///
/// 「配列だけ出せ」と言っても囲ってくることがあるので、**最初の`[`から最後の`]`
/// まで**を取る。長さが合わなければ読めなかった扱い。
#[must_use]
pub fn read_claude(raw: &str, want: usize) -> Option<Vec<String>> {
    let start = raw.find('[')?;
    let end = raw.rfind(']')?;
    if start >= end {
        return None;
    }
    let parsed: Vec<String> = serde_json::from_str(&raw[start..=end]).ok()?;
    (parsed.len() == want).then_some(parsed)
}

/// `T`の精訳。オンデバイスの訳は速いが粗く、技術記事だと判断を誤る訳が混ざる
/// ——ちゃんと読むと決めた頁だけclaudeへ投げ直すための口。
///
/// # Errors
/// CLIが居ない・起こせない・返事を読めない・時間切れのとき。
pub fn translate_with_claude(texts: &[String]) -> Result<Vec<String>, String> {
    if texts.is_empty() {
        return Err(crate::tr!("Nothing to translate", "訳す本文が無い").to_string());
    }
    if texts.len() > CLAUDE_MAX_TEXTS || texts.iter().map(String::len).sum::<usize>() > CLAUDE_MAX_BYTES {
        return Err(crate::tr!("Too much text to translate at once", "一度に訳す本文が多すぎる").to_string());
    }
    let bin = claude_bin();
    if !bin.is_file() {
        return Err(crate::tr!("claude CLI not found", "claude CLI が見つからない").to_string());
    }
    let json =
        serde_json::to_string(texts).map_err(|_| "can't encode the text".to_string())?;
    // 本文は外の頁のもの。道具を全部切り、利用者の設定(CLAUDE.md・hooks・MCP)も
    // 読ませず、空の一時フォルダで起こす——本文に仕込まれた指示が手元に届かないように。
    let dir = std::env::temp_dir().join("armature-translate");
    std::fs::create_dir_all(&dir).map_err(|_| "can't prepare a folder for claude".to_string())?;
    let _turn = CLAUDE_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut cmd = Command::new(&bin);
    crate::proc::child_env(&mut cmd);
    // 題(prompt)は `--tools` より前に置く。後ろに置くと `--tools` の値として食われる。
    cmd.arg("-p")
        .arg(claude_prompt(&json))
        .arg("--model")
        .arg("sonnet")
        .arg("--tools")
        .arg("")
        .arg("--safe-mode")
        .arg("--no-session-persistence")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|_| crate::tr!("Couldn't start claude", "claudeを起動できなかった").to_string())?;
    // 返ってこないまま居座らせない。待つのはこのスレッドだけで、窓は動き続ける。
    let until = Instant::now() + CLAUDE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(_) => return Err("lost track of claude".to_string()),
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            return Err(crate::tr!("Claude's translation timed out", "精訳が時間切れになった").to_string());
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    let out = child
        .wait_with_output()
        .map_err(|_| crate::tr!("Couldn't read Claude's translation", "精訳の応答を読めなかった").to_string())?;
    let raw = String::from_utf8_lossy(&out.stdout);
    read_claude(&raw, texts.len())
        .ok_or_else(|| crate::tr!("Couldn't read Claude's translation", "精訳の応答を読めなかった").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese_pages_are_left_alone() {
        assert!(is_japanese("ja"));
        assert!(is_japanese("ja-JP"));
        assert!(!is_japanese("en"));
        assert!(!is_japanese(""));
        assert!(!is_japanese("jv"), "ジャワ語まで弾いている");
    }

    #[test]
    fn automatic_translation_requires_an_app_owned_engine() {
        let error = translate(&["Hello, world.".to_string()]).expect_err("共有層は訳さない");
        assert!(error.contains("Apple Translation"), "{error}");
    }

    #[test]
    fn claude_may_wrap_the_array_in_prose_and_we_still_find_it() {
        let raw = "はい、訳しました:\n```json\n[\"一行目\",\"二行目\"]\n```\n";
        assert_eq!(
            read_claude(raw, 2),
            Some(vec!["一行目".to_string(), "二行目".to_string()])
        );
    }

    #[test]
    fn a_claude_reply_of_the_wrong_length_is_thrown_away() {
        assert!(read_claude(r#"["一行目"]"#, 2).is_none());
        for junk in ["", "訳せませんでした", "[", "]["] {
            assert!(read_claude(junk, 1).is_none(), "{junk}");
        }
    }

    #[test]
    fn the_prompt_carries_the_input_and_asks_for_nothing_else() {
        let _ja = crate::lang::scoped(crate::lang::Lang::Ja);
        let prompt = claude_prompt(r#"["Hello"]"#);
        assert!(prompt.contains(r#"["Hello"]"#), "{prompt}");
        assert!(prompt.contains("同じ長さ・同じ順序"), "{prompt}");
    }

    #[test]
    fn nothing_to_translate_is_refused_before_translation() {
        assert!(translate(&[]).is_err());
        assert!(translate_with_claude(&[]).is_err());
    }

    #[test]
    fn a_page_cannot_hand_claude_more_than_one_batch() {
        let many = vec!["Hello".to_string(); CLAUDE_MAX_TEXTS + 1];
        let error = translate_with_claude(&many).expect_err("上限を越えた束は起こさない");
        assert!(error.contains("Too much") || error.contains("多すぎる"), "{error}");
    }
}
