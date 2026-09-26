//! 状態の置き場への薄い口。正本は [`crate::paths::state`]。

use std::path::PathBuf;

/// 状態ディレクトリ。正本は [`crate::paths::state`]。
#[must_use]
pub fn state_dir() -> PathBuf {
    crate::paths::state()
}
