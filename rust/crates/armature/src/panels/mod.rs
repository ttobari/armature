//! The panels Armature starts with. Each one is a plain [`crate::Panel`], written the same way as a
//! panel of your own — read them as examples, leave any of them out of your build
//! ([`crate::App::without`]), or replace one by giving yours the same key.

mod browser_tabs;
mod calendar;
mod clock;
mod music;
mod sessions;
mod tasks;

pub use browser_tabs::BrowserTabs;
pub use calendar::Calendar;
pub use clock::Clock;
pub use music::Music;
pub use sessions::Sessions;
pub use tasks::Tasks;
