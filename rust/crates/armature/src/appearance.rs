//! Which color theme the window wears. The choice is saved as `theme` in the state folder;
//! without one the window starts in Catppuccin Frappé.

use crate::palette::{self, Theme};

/// Sets the saved theme at startup.
pub fn init() {
    if let Some(theme) = stored() {
        palette::set_theme(theme);
    }
}

/// The user picked a theme in the settings screen. Takes effect at once and is saved.
pub fn choose(theme: Theme) {
    palette::set_theme(theme);
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{}\n", theme.key()));
}

fn path() -> std::path::PathBuf {
    armature_core::paths::state().join("theme")
}

fn stored() -> Option<Theme> {
    Theme::from_key(&std::fs::read_to_string(path()).ok()?)
}
