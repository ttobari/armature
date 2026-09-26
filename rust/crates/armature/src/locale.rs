//! Which language the app speaks, and whether the calendar marks Japanese holidays.
//!
//! The language is the saved choice (`language` in the state folder) if there is one, else the
//! Mac's first preferred language (English unless it is Japanese). Holidays follow the Mac's
//! region: a Mac set to Japan marks them whatever the language.

use armature_core::lang::{self, Lang};

/// Sets the language and the holiday switch at startup.
pub fn init() {
    let lang = stored().unwrap_or_else(system_language);
    lang::set(lang);
    lang::set_japan_holidays(region_is_japan() || lang == Lang::Ja);
}

/// The user picked a language in the settings screen. Takes effect at once and is saved.
pub fn choose(lang: Lang) {
    lang::set(lang);
    lang::set_japan_holidays(region_is_japan() || lang == Lang::Ja);
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{}\n", lang.key()));
}

fn path() -> std::path::PathBuf {
    armature_core::paths::state().join("language")
}

fn stored() -> Option<Lang> {
    Lang::from_tag(std::fs::read_to_string(path()).ok()?.trim())
}

/// The Mac's first preferred language, if it is one the app speaks.
fn system_language() -> Lang {
    let preferred = objc2_foundation::NSLocale::preferredLanguages();
    preferred
        .firstObject()
        .and_then(|tag| Lang::from_tag(&tag.to_string()))
        .unwrap_or(Lang::En)
}

fn region_is_japan() -> bool {
    objc2_foundation::NSLocale::currentLocale()
        .regionCode()
        .is_some_and(|code| code.to_string() == "JP")
}
