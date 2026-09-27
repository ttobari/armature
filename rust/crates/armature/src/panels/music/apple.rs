//! Apple Music のパネル。いま鳴っている曲を出し、手元から止めたり送ったりする。
//!
//! **見張りは Music.app を起こさない。**`application "Music" is running` を先に
//! 見て、起きていなければ何も触らない——飾りのパネルのために音楽アプリが立ち上がる
//! のは、利用者が求めた「常時見えていてほしい」の逆をやることになる。
//!
//! **起こすのは利用者がパネルを開いたときだけ**。
//! 定期の見張り([`snapshot`])は今までどおり起きているときしか触らない。
//!
//! 見えるのはライブラリに入れた曲とプレイリストだけ(AppleScript の範囲)。
//! カタログの検索はここからは届かない。

use armature_core::tr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 再生の状態。`Off` は Music が起きていない。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Off,
    Stopped,
    Playing,
    Paused,
}

impl State {
    fn from_key(value: &str) -> Self {
        match value {
            "playing" => Self::Playing,
            "paused" => Self::Paused,
            "stopped" => Self::Stopped,
            _ => Self::Off,
        }
    }

    /// 曲が載っているか。止まっているときは題も作者も無い。
    #[must_use]
    pub fn has_track(self) -> bool {
        matches!(self, Self::Playing | Self::Paused)
    }
}

/// いまの一枚。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Now {
    pub state: State,
    pub name: String,
    pub artist: String,
    pub album: String,
    pub shuffle: bool,
    /// `off` / `one` / `all`。
    pub repeat: String,
    /// Music.app 自身の音量(0〜100)。**機体全体の音量とは別物**——ここを
    /// 動かしても他のアプリの音は変わらない。
    pub volume: u8,
    /// アートワークを書き出した先。曲が変わるたびに書き直す。
    pub artwork: Option<PathBuf>,
}

impl Now {
    /// 曲の見分け。ここが変わったときだけ絵を取り直す。
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}\u{1}{}", self.name, self.artist)
    }
}

/// パネルから出せる指図。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command_ {
    PlayPause,
    Next,
    Previous,
    Shuffle(bool),
    /// `off` / `one` / `all`。
    Repeat(String),
    Play(String),
    /// 0〜100。範囲外は Music 側が丸める前にこちらで潰す。
    Volume(u8),
}

const SNAPSHOT: &str = r#"
if application "Music" is not running then
  return "off"
end if
tell application "Music"
  set out to (player state as text)
  if player state is playing or player state is paused then
    set t to current track
    set out to out & linefeed & (name of t) & linefeed & (artist of t) & linefeed & (album of t)
  else
    set out to out & linefeed & "" & linefeed & "" & linefeed & ""
  end if
  set out to out & linefeed & (shuffle enabled as text) & linefeed & (song repeat as text)
  set out to out & linefeed & (sound volume as text)
  return out
end tell
"#;

/// プレイリストの名簿だけを取る。**見張り(`SNAPSHOT`)からは外してある**——
/// 全プレイリストの走査は 27 本で 0.53 秒掛かり、5 秒ごとの見張りに載せると
/// 窓が立った直後まで Music を掴み続ける(2026-08-30 実測。名簿抜きなら 0.17 秒)。
/// 要るのは選択画面を開いたときだけなので、そのときだけ訊きに行く。
const PLAYLISTS: &str = r#"
if application "Music" is not running then
  return ""
end if
tell application "Music"
  -- 変数名に `names` は使えない。Music 側の用語と当たって
  -- 「constant eSrAkSrS を "" に設定できません(-10003)」で落ちる(2026-08-22実測)。
  set titles to ""
  repeat with p in (every user playlist)
    set titles to titles & (name of p) & tab
  end repeat
  return titles
end tell
"#;

/// プレイリストの名簿。Music が寝ていれば空。
#[must_use]
pub fn playlists() -> Vec<String> {
    run(PLAYLISTS)
        .unwrap_or_default()
        .split('\t')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// いまの一枚を読む。Music が起きていなければ [`State::Off`] を返すだけ。
#[must_use]
pub fn snapshot() -> Now {
    let Some(text) = run(SNAPSHOT) else {
        return Now::default();
    };
    let mut lines = text.lines();
    let state = State::from_key(lines.next().unwrap_or("off").trim());
    let mut now = Now {
        state,
        name: lines.next().unwrap_or_default().trim().to_string(),
        artist: lines.next().unwrap_or_default().trim().to_string(),
        album: lines.next().unwrap_or_default().trim().to_string(),
        ..Now::default()
    };
    now.shuffle = lines.next().unwrap_or("false").trim() == "true";
    now.repeat = lines.next().unwrap_or("off").trim().to_string();
    // 読めない機体では 100 に倒す。0 に倒すと「音が出ない」を既定にしてしまう。
    now.volume = lines
        .next()
        .and_then(|line| line.trim().parse::<i32>().ok())
        .map_or(100, |value| value.clamp(0, 100) as u8);
    now
}

/// 指図を1つ送る。Music が起きていなければ何もしない。
/// Music.app を起こす。**パネルを開いた時だけ**呼ぶ。
///
/// `activate` ではなく `launch` を使う——`activate` は Music を前面に出すので、
/// コックピットの窓が後ろへ落ちる。`launch` は起こすだけで前面は譲らない。
/// 既に起きているときは何もしない(冪等)。
pub fn launch() {
    let _ = run("if application \"Music\" is not running then\n  tell application \"Music\" to launch\nend if");
}

pub fn send(command: &Command_) -> Result<(), String> {
    let body = match command {
        Command_::PlayPause => "playpause".to_string(),
        Command_::Next => "next track".to_string(),
        Command_::Previous => "previous track".to_string(),
        Command_::Shuffle(on) => format!("set shuffle enabled to {on}"),
        Command_::Repeat(mode) => format!("set song repeat to {mode}"),
        Command_::Play(name) => format!("play playlist \"{}\"", escape(name)),
        Command_::Volume(level) => format!("set sound volume to {}", (*level).min(100)),
    };
    let script = format!(
        "if application \"Music\" is not running then\n  return\nend if\ntell application \"Music\" to {body}"
    );
    run(&script).map(|_| ()).ok_or_else(|| tr!("Can't reach Music", "Music へ届かない").to_string())
}

/// アートワークを書き出す。載っていない曲では `None`。
#[must_use]
pub fn artwork(target: &Path) -> Option<PathBuf> {
    let path = target.to_string_lossy().to_string();
    let script = format!(
        r#"
if application "Music" is not running then
  return "no"
end if
tell application "Music"
  if player state is stopped then
    return "no"
  end if
  try
    set d to raw data of artwork 1 of current track
  on error
    return "no"
  end try
end tell
set f to open for access POSIX file "{path}" with write permission
set eof f to 0
write d to f
close access f
return "yes"
"#
    );
    (run(&script)?.trim() == "yes").then(|| target.to_path_buf())
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn run(script: &str) -> Option<String> {
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_player_has_no_track() {
        assert!(!State::Off.has_track());
        assert!(!State::Stopped.has_track());
        assert!(State::Playing.has_track());
        assert!(State::Paused.has_track());
    }

    #[test]
    fn quotes_in_a_playlist_name_do_not_break_the_script() {
        assert_eq!(escape(r#"Rock "n" Roll"#), r#"Rock \"n\" Roll"#);
        assert_eq!(escape(r"back\slash"), r"back\\slash");
    }

    #[test]
    fn the_track_key_changes_with_the_song() {
        let first = Now {
            name: "Ash".into(),
            artist: "Fripp".into(),
            ..Now::default()
        };
        let second = Now {
            name: "Ash".into(),
            artist: "Eno".into(),
            ..Now::default()
        };
        assert_ne!(first.key(), second.key());
    }

    /// Music が起きていない機体でも、読むだけなら通る(この機体の実測)。
    ///
    /// 本物の Music へ osascript で訊くので、既定では走らせない。画面のセッションが無い
    /// 機体(ssh で入った VM)では Apple Events の応答を 2 分待って試験全体が止まる。
    /// Mac の前で `scripts/test.sh -p armature -- --ignored reading_without_music` で走らせる。
    #[test]
    #[ignore = "talks to the real Music.app"]
    fn reading_without_music_running_is_safe() {
        let now = snapshot();
        if now.state == State::Off {
            assert!(now.name.is_empty());
        }
    }
}
