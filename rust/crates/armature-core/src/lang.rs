//! The display language: English or Japanese.
//!
//! One value for the whole process. The app picks it at startup (the saved choice, else the
//! Mac's preferred language) and changes it from the settings screen. Text is written in pairs
//! right where it is used — `tr!("Tasks", "タスク")` — so a panel carries its own words and
//! nothing has to be looked up by key.
//!
//! Tests can switch the language for their own thread only ([`scoped`]), so tests that check
//! Japanese text do not disturb tests running beside them.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// A display language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Ja,
}

impl Lang {
    /// Every language, in the order the settings screen lists them.
    pub const ALL: [Self; 2] = [Self::En, Self::Ja];

    /// The name written to the settings file.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Ja => "ja",
        }
    }

    /// Reads `en` / `ja` and language tags such as `en-US`, `ja-JP` or `ja_JP`.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        let primary = tag.trim().split(['-', '_']).next()?.to_ascii_lowercase();
        match primary.as_str() {
            "en" => Some(Self::En),
            "ja" => Some(Self::Ja),
            _ => None,
        }
    }

    /// The name shown in the settings screen, always written in the language itself.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Ja => "日本語",
        }
    }

    /// Position in [`Self::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|lang| *lang == self).unwrap_or(0)
    }

    /// Back from a position. Out of range stops at the last one.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::En => 0,
            Self::Ja => 1,
        }
    }

    fn from_bits(bits: u8) -> Self {
        if bits == 1 { Self::Ja } else { Self::En }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);
static JAPAN_HOLIDAYS: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// A test's own language (see [`scoped`]). Nothing else sets it.
    static OVERRIDE: Cell<Option<Lang>> = const { Cell::new(None) };
}

/// The language the app speaks now.
#[must_use]
pub fn current() -> Lang {
    OVERRIDE
        .with(Cell::get)
        .unwrap_or_else(|| Lang::from_bits(CURRENT.load(Ordering::Relaxed)))
}

/// Switches the language for the whole app.
pub fn set(lang: Lang) {
    CURRENT.store(lang.to_bits(), Ordering::Relaxed);
}

/// Picks the text for the current language. Used through [`tr!`](crate::tr).
#[must_use]
pub fn pick<T>(en: T, ja: T) -> T {
    match current() {
        Lang::En => en,
        Lang::Ja => ja,
    }
}

/// Whether the calendar marks Japanese public holidays. That follows the Mac's region, not
/// the display language: a Mac set to Japan shows them in English too.
#[must_use]
pub fn japan_holidays() -> bool {
    JAPAN_HOLIDAYS.load(Ordering::Relaxed)
}

pub fn set_japan_holidays(on: bool) {
    JAPAN_HOLIDAYS.store(on, Ordering::Relaxed);
}

/// Speaks `lang` on this thread until the guard is dropped. For tests.
#[must_use]
pub fn scoped(lang: Lang) -> Scoped {
    let previous = OVERRIDE.with(|cell| cell.replace(Some(lang)));
    Scoped { previous }
}

/// Puts the thread's language back when dropped (see [`scoped`]).
pub struct Scoped {
    previous: Option<Lang>,
}

impl Drop for Scoped {
    fn drop(&mut self) {
        OVERRIDE.with(|cell| cell.set(self.previous));
    }
}

/// `tr!("English", "日本語")` — the text for the current display language.
///
/// Both sides are evaluated, so `tr!(format!(…), format!(…))` works too.
#[macro_export]
macro_rules! tr {
    ($en:expr, $ja:expr $(,)?) => {
        $crate::lang::pick($en, $ja)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_read_by_their_first_part() {
        assert_eq!(Lang::from_tag("en-US"), Some(Lang::En));
        assert_eq!(Lang::from_tag("ja_JP"), Some(Lang::Ja));
        assert_eq!(Lang::from_tag("JA"), Some(Lang::Ja));
        assert_eq!(Lang::from_tag("fr-FR"), None);
        assert_eq!(Lang::from_tag(""), None);
    }

    #[test]
    fn a_scoped_language_stays_on_its_thread() {
        {
            let _ja = scoped(Lang::Ja);
            assert_eq!(tr!("Tasks", "タスク"), "タスク");
            let other = std::thread::spawn(current).join().expect("thread");
            assert_eq!(other, Lang::from_bits(CURRENT.load(Ordering::Relaxed)));
        }
        let _en = scoped(Lang::En);
        assert_eq!(tr!("Tasks", "タスク"), "Tasks");
    }

    #[test]
    fn positions_round_trip() {
        for lang in Lang::ALL {
            assert_eq!(Lang::from_index(lang.index()), lang);
            assert_eq!(Lang::from_tag(lang.key()), Some(lang));
        }
    }
}
