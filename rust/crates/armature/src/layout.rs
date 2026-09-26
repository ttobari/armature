//! Panels and where they sit.
//!
//! Only the Claude terminal in the middle of the window is fixed; everything else is a panel.
//! Which panels are shown, and in which column in what order, is decided by one file,
//! `panels.conf`:
//!
//! ```text
//! left = browser, sessions
//! right = clock, tasks
//! ```
//!
//! Each line is one column, top to bottom. A panel that is not listed is not shown, and does no
//! work in the background either. You or Claude edit the file, and the window notices the change
//! and rearranges itself on the spot.
//!
//! The file lives in the Armature folder (`~/Armature`), not in the hidden state folder: Claude
//! sessions start there, so "move the clock to the left" finds it.
//!
//! Every panel — the ones Armature comes with and yours — is a [`crate::Panel`] registered with
//! [`crate::App`]; the layout knows each by its place in that list ([`PanelId`]) and its
//! [`CustomInfo`].
//!
//! Two builds can share the file (plain Armature and one with panels of its own, when neither is
//! given a name of its own). A name this build does not know is not shown, but it is kept where
//! it was when the file is written back ([`Stranger`]), so rearranging panels in one build does
//! not drop the other's.

use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};


/// What the layout knows of a registered panel (see [`crate::Panel`]).
#[derive(Clone, Copy, Debug)]
pub struct CustomInfo {
    pub key: &'static str,
    pub fills: bool,
    pub side: Side,
    /// Whether the card leaves a margin around the panel's content.
    pub padded: bool,
    /// Whether it is in the arrangement Armature starts with.
    pub shown: bool,
    /// Whether it takes the keyboard when clicked.
    pub keys: bool,
    /// The letter that, with ⌘, gives it the keyboard.
    pub shortcut: Option<char>,
}

static CUSTOM: OnceLock<Vec<CustomInfo>> = OnceLock::new();

/// Registers the panels. Called once, before the window opens.
pub fn register(custom: Vec<CustomInfo>) {
    let _ = CUSTOM.set(custom);
}

fn custom() -> &'static [CustomInfo] {
    CUSTOM.get().map_or(&[], Vec::as_slice)
}

/// One panel, by its place among the registered panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PanelId(pub u16);

impl PanelId {
    /// Every registered panel, in the order they were registered.
    #[must_use]
    pub fn all() -> Vec<Self> {
        (0..custom().len())
            .filter_map(|index| u16::try_from(index).ok())
            .map(Self)
            .collect()
    }

    #[must_use]
    pub fn info(self) -> Option<&'static CustomInfo> {
        custom().get(usize::from(self.0))
    }

    #[must_use]
    pub fn key(self) -> &'static str {
        self.info().map_or("", |info| info.key)
    }

    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        Self::all().into_iter().find(|panel| panel.key() == key)
    }

    #[must_use]
    pub fn fills(self) -> bool {
        self.info().is_some_and(|info| info.fills)
    }

    /// Whether the panel wants a margin around its content.
    #[must_use]
    pub fn padded(self) -> bool {
        self.info().is_none_or(|info| info.padded)
    }
}

/// A column. More places may come, so match it with a `_` arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Side {
    Left,
    Right,
}

/// Which panels sit in which column, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub left: Vec<PanelId>,
    pub right: Vec<PanelId>,
    /// Names in the file this build does not know. Kept for writing the file back.
    strangers: Vec<Stranger>,
}

/// A name in `panels.conf` that this build does not know: most likely a panel of another build
/// that shares the file. It is not shown, and it goes back where it was when the file is written.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stranger {
    key: String,
    side: Side,
    /// The known panels before it on its line, in order. It is written after the last of them
    /// still in its column, or at the top of the column when none is.
    after: Vec<PanelId>,
}

/// The panels Armature starts with, by key, in the order they stand in their columns.
const DEFAULT_LEFT: [&str; 2] = ["browser", "sessions"];
/// The calendar and the music player are not in it: add `calendar` to `panels.conf` and it sits
/// under the clock; add `music` to the left line for Apple Music.
const DEFAULT_RIGHT: [&str; 2] = ["clock", "tasks"];

impl Default for Layout {
    /// Armature's own arrangement, with the other panels at the end of their columns.
    fn default() -> Self {
        let by_key = |keys: &[&str]| keys.iter().filter_map(|key| PanelId::from_key(key)).collect();
        let mut layout = Self {
            left: by_key(&DEFAULT_LEFT),
            right: by_key(&DEFAULT_RIGHT),
            strangers: Vec::new(),
        };
        for (index, info) in custom().iter().enumerate() {
            let placed = DEFAULT_LEFT.contains(&info.key)
                || DEFAULT_RIGHT.contains(&info.key)
                || info.key == "calendar"
                || info.key == "music";
            if !placed && info.shown {
                layout
                    .column_mut(info.side)
                    .push(PanelId(index as u16));
            }
        }
        layout
    }
}

impl Layout {
    /// The file (directly in the Armature folder).
    #[must_use]
    pub fn path() -> PathBuf {
        armature_core::paths::tasks_home().join("panels.conf")
    }

    /// Reads the file. No file means the default arrangement.
    #[must_use]
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Reads the `left = …` and `right = …` lines; anything after `#` is a comment. Unknown
    /// names are not shown but kept for writing the file back, and a panel named twice keeps its
    /// first place. A file with neither line reads as the default arrangement, so an empty file
    /// does not hide everything.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut left = None;
        let mut right = None;
        let mut strangers: Vec<Stranger> = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let side = match key.trim() {
                "left" => Side::Left,
                "right" => Side::Right,
                _ => continue,
            };
            let mut panels = Vec::new();
            for name in value.split(',').map(str::trim).filter(|name| !name.is_empty()) {
                match PanelId::from_key(name) {
                    Some(panel) => panels.push(panel),
                    None if !strangers.iter().any(|stranger| stranger.key == name) => {
                        strangers.push(Stranger {
                            key: name.to_string(),
                            side,
                            after: panels.clone(),
                        });
                    }
                    None => {}
                }
            }
            match side {
                Side::Left => left = Some(panels),
                Side::Right => right = Some(panels),
            }
        }
        if left.is_none() && right.is_none() {
            return Self::default();
        }
        let mut layout = Self {
            left: left.unwrap_or_default(),
            right: right.unwrap_or_default(),
            strangers,
        };
        layout.dedup();
        layout
    }

    /// The names written on one column's line: its panels, with the names this build does not
    /// know put back after the panel they followed.
    fn names(&self, side: Side) -> Vec<&str> {
        let column = match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        };
        let strangers: Vec<&Stranger> = self
            .strangers
            .iter()
            .filter(|stranger| stranger.side == side)
            .collect();
        let anchor = |stranger: &Stranger| {
            stranger
                .after
                .iter()
                .rev()
                .find(|panel| column.contains(panel))
                .copied()
        };
        let mut names: Vec<&str> = strangers
            .iter()
            .filter(|stranger| anchor(stranger).is_none())
            .map(|stranger| stranger.key.as_str())
            .collect();
        for panel in column {
            names.push(panel.key());
            names.extend(
                strangers
                    .iter()
                    .filter(|stranger| anchor(stranger) == Some(*panel))
                    .map(|stranger| stranger.key.as_str()),
            );
        }
        names
    }

    fn dedup(&mut self) {
        let mut seen = Vec::new();
        self.left.retain(|panel| {
            let fresh = !seen.contains(panel);
            seen.push(*panel);
            fresh
        });
        self.right.retain(|panel| {
            let fresh = !seen.contains(panel);
            seen.push(*panel);
            fresh
        });
    }

    /// What goes into the file.
    #[must_use]
    pub fn render(&self) -> String {
        let all = PanelId::all()
            .into_iter()
            .map(PanelId::key)
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "# Armature panels. Each line lists one column, top to bottom. Panels not listed are hidden.\n\
             # Names: {all}\n\
             left = {}\n\
             right = {}\n",
            self.names(Side::Left).join(", "),
            self.names(Side::Right).join(", "),
        )
    }

    /// Writes the file. The window works without it, so a failure is ignored.
    pub fn store(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, self.render());
    }

    /// Whether a panel is shown.
    #[must_use]
    pub fn shows(&self, panel: PanelId) -> bool {
        self.side_of(panel).is_some()
    }

    #[must_use]
    pub fn side_of(&self, panel: PanelId) -> Option<Side> {
        if self.left.contains(&panel) {
            Some(Side::Left)
        } else if self.right.contains(&panel) {
            Some(Side::Right)
        } else {
            None
        }
    }

    fn column_mut(&mut self, side: Side) -> &mut Vec<PanelId> {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }
}

/// The current arrangement. The window geometry (where the browser page goes) and the settings
/// screen read it without holding the app's state.
static CURRENT: RwLock<Option<Layout>> = RwLock::new(None);

/// Announces the current arrangement. Call it after every change.
pub fn publish(layout: &Layout) {
    if let Ok(mut slot) = CURRENT.write() {
        *slot = Some(layout.clone());
    }
}

/// The current arrangement, or the default one before anything was announced.
#[must_use]
pub fn current() -> Layout {
    CURRENT
        .read()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(key: &'static str, side: Side, fills: bool) -> CustomInfo {
        CustomInfo {
            key,
            fills,
            side,
            padded: true,
            shown: true,
            keys: false,
            shortcut: None,
        }
    }

    /// Every test sees the same registry: a panel of the user's on the right, then the clock and
    /// the task list (two of Armature's own).
    fn registered() {
        register(vec![
            info("weather", Side::Right, false),
            info("clock", Side::Right, false),
            info("tasks", Side::Right, true),
        ]);
    }

    const WEATHER: PanelId = PanelId(0);
    const CLOCK: PanelId = PanelId(1);
    const TASKS: PanelId = PanelId(2);

    #[test]
    fn the_file_round_trips() {
        registered();
        let layout = Layout {
            left: vec![WEATHER],
            right: vec![TASKS, CLOCK],
            strangers: Vec::new(),
        };
        assert_eq!(Layout::parse(&layout.render()), layout);
        assert_eq!(Layout::parse(&Layout::default().render()), Layout::default());
    }

    #[test]
    fn unknown_names_are_hidden_and_repeats_are_skipped() {
        registered();
        let layout = Layout::parse("left = clock, radio, clock\nright = clock, tasks, weather\n");
        assert_eq!(layout.left, vec![CLOCK]);
        assert_eq!(layout.right, vec![TASKS, WEATHER]);
        assert!(layout.render().contains("left = clock, radio\n"), "radio is kept in the file");
    }

    /// Two builds can share the file: names this one does not know are kept where they were.
    #[test]
    fn another_builds_panels_survive_in_this_one() {
        registered();
        let layout =
            Layout::parse("left = counter, sessions\nright = clock, radio, calendar, tasks\n");
        assert!(layout.left.is_empty());
        assert_eq!(layout.right, vec![CLOCK, TASKS]);
        assert!(layout.render().contains("left = counter, sessions\n"));
        assert!(layout.render().contains("right = clock, radio, calendar, tasks\n"));
        let again = Layout::parse(&layout.render());
        assert_eq!(again.render(), layout.render());
    }

    #[test]
    fn an_empty_line_empties_that_column_but_an_empty_file_does_not() {
        registered();
        let layout = Layout::parse("left =\nright = tasks\n");
        assert!(layout.left.is_empty());
        assert_eq!(layout.right, vec![TASKS]);
        assert_eq!(Layout::parse(""), Layout::default());
        assert_eq!(Layout::parse("# just a comment\n"), Layout::default());
    }

    /// Armature's own panels take their places; the others join the end of their column.
    #[test]
    fn the_default_arrangement_puts_armatures_panels_first() {
        registered();
        let layout = Layout::default();
        assert_eq!(layout.right, vec![CLOCK, TASKS, WEATHER]);
        assert_eq!(PanelId::from_key("weather"), Some(WEATHER));
        assert_eq!(PanelId(9).key(), "", "an unregistered index names nothing");
    }
}
