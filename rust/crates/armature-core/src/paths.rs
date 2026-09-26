//! このアプリの身元と置き場の正本。
//!
//! 置き場は**起動の最初にアプリが [`set`] で渡す**。環境変数は読まない——利用者の
//! シェルに別の道具の変数が残っていても、それに引きずられて他人のファイルを
//! 書き換えないため。
//!
//! 身元(名前と slug)は外の crate が `App::name` で替えられる。替えた版は状態の置き場・
//! タスクの置き場・tmux の口を素の Armature と分ける。替えなければ素の Armature と同じ。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 素の Armature の名前(窓の題・タスクの置き場のフォルダ名)。
pub const DEFAULT_NAME: &str = "Armature";

/// 素の Armature の slug(状態の置き場・tmux の口)。
pub const DEFAULT_SLUG: &str = "armature";

static IDENTITY: OnceLock<(String, String)> = OnceLock::new();
static TASKS_HOME: OnceLock<PathBuf> = OnceLock::new();
static STATE: OnceLock<PathBuf> = OnceLock::new();

/// 身元を据える。**置き場を読むより前に1度だけ。**2度目以降は無視して `false`。
pub fn set_identity(name: &str, slug: &str) -> bool {
    IDENTITY.set((name.to_string(), slug.to_string())).is_ok()
}

/// 名前(窓の題・`~/<名前>`)。
#[must_use]
pub fn name() -> &'static str {
    IDENTITY.get().map_or(DEFAULT_NAME, |(name, _)| name.as_str())
}

/// slug(`~/Library/Application Support/<slug>`・`tmux -L <slug>`)。
#[must_use]
pub fn slug() -> &'static str {
    IDENTITY.get().map_or(DEFAULT_SLUG, |(_, slug)| slug.as_str())
}

/// slug に使える綴りか。小文字・数字・`-` だけで、空でない。
#[must_use]
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 名前に使える綴りか。ホームの下のフォルダ名になるので、`/` や `.` だけの名は断る。
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && trimmed == name
        && !name.contains(['/', ':', '\0'])
        && name != "."
        && name != ".."
}

/// 起動の最初に1度だけ呼ぶ。2度目以降は無視される。
pub fn set(tasks_home: PathBuf, state: PathBuf) {
    let _ = TASKS_HOME.set(tasks_home);
    let _ = STATE.set(state);
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// `slug` の版の状態の置き場。
#[must_use]
pub fn state_for(home: &Path, slug: &str) -> PathBuf {
    home.join("Library/Application Support").join(slug)
}

/// `name` の版のタスクの置き場の親。
#[must_use]
pub fn tasks_home_for(home: &Path, name: &str) -> PathBuf {
    home.join(name)
}

/// 既定の状態の置き場(`~/Library/Application Support/<slug>`)。
#[must_use]
pub fn default_state() -> PathBuf {
    state_for(&home(), slug())
}

/// 既定のタスクの置き場の親(`~/<名前>`)。タスクは `<ここ>/tasks/<ID>.md`。
#[must_use]
pub fn default_tasks_home() -> PathBuf {
    tasks_home_for(&home(), name())
}

/// 状態(ログ・ブラウザの控え・設定・暦の印)の置き場。
#[must_use]
pub fn state() -> PathBuf {
    STATE.get().cloned().unwrap_or_else(default_state)
}

/// タスクの置き場の親。
#[must_use]
pub fn tasks_home() -> PathBuf {
    TASKS_HOME.get().cloned().unwrap_or_else(default_tasks_home)
}

/// タスクの md が並ぶフォルダ。
#[must_use]
pub fn tasks_dir() -> PathBuf {
    tasks_home().join("tasks")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_build_keeps_its_folders_apart_from_plain_armature() {
        let home = Path::new("/Users/x");
        assert_eq!(
            state_for(home, DEFAULT_SLUG),
            PathBuf::from("/Users/x/Library/Application Support/armature")
        );
        assert_eq!(tasks_home_for(home, DEFAULT_NAME), PathBuf::from("/Users/x/Armature"));
        assert_eq!(
            state_for(home, "my-armature"),
            PathBuf::from("/Users/x/Library/Application Support/my-armature")
        );
        assert_eq!(tasks_home_for(home, "My Armature"), PathBuf::from("/Users/x/My Armature"));
    }

    #[test]
    fn slugs_and_names_are_checked() {
        assert!(is_valid_slug("my-armature2"));
        for bad in ["", "My", "my armature", "my_armature", "../x", "ゲッソ"] {
            assert!(!is_valid_slug(bad), "{bad:?}");
        }
        assert!(is_valid_name("My Armature"));
        assert!(is_valid_name("私の Armature"));
        for bad in ["", " x", "x ", "a/b", "a:b", ".", ".."] {
            assert!(!is_valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn without_a_name_the_build_is_plain_armature() {
        // 試験は身元を据えない。据えていなければ素の名と slug が返る。
        if IDENTITY.get().is_none() {
            assert_eq!(name(), DEFAULT_NAME);
            assert_eq!(slug(), DEFAULT_SLUG);
        }
    }
}
