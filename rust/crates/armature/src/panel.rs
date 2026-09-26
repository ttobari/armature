//! Panels: every panel in the window, built in or your own, is a [`Panel`].
//!
//! Write a type that implements it, hand it to [`crate::App::panel`], and it sits in the window
//! like the built-in ones (which are written the same way — see [`crate::panels`]): it can sit in
//! either column, is saved in `panels.conf` under its [`Panel::KEY`], and its
//! [`Panel::subscription`] runs only while it is shown.
//!
//! What a panel can do:
//!
//! - draw itself ([`Panel::view`]) and handle its own messages ([`Panel::update`])
//! - hear what Armature knows ([`Panel::notice`]): the terminal's tabs, the browser's tabs
//! - take the keyboard ([`Panel::KEYS`], [`Panel::key`]), reached with ⌘ + [`Panel::SHORTCUT`]
//! - borrow the center of the window ([`Panel::center`])
//! - tell Claude about itself ([`Panel::note`])
//! - ask Armature for things ([`Host`]): open a page, start Claude, switch or reorder tabs
//!
//! ```no_run
//! use armature::iced::Task;
//! use armature::iced::widget::{button, column, text};
//! use armature::{App, Element, Host, Panel, tr};
//!
//! #[derive(Default)]
//! struct Hello {
//!     note: String,
//! }
//!
//! #[derive(Clone, Debug)]
//! enum Message {
//!     Docs,
//!     Opened(Result<(), String>),
//! }
//!
//! impl Panel for Hello {
//!     const KEY: &'static str = "hello";
//!     type Message = Message;
//!
//!     fn view(&self) -> Element<'_, Message> {
//!         column![
//!             text(tr!("Hello from my own panel", "自分のパネルから")),
//!             button(tr!("Docs", "資料")).on_press(Message::Docs),
//!             text(&self.note),
//!         ]
//!         .into()
//!     }
//!
//!     fn update(&mut self, message: Message, host: &mut Host) -> Task<Message> {
//!         match message {
//!             // Fire and forget: `host.open_url(…);` is enough. `map` also hears how it went.
//!             Message::Docs => host.open_url("https://docs.rs/iced").map(Message::Opened),
//!             Message::Opened(outcome) => {
//!                 self.note = outcome.err().unwrap_or_default();
//!                 Task::none()
//!             }
//!         }
//!     }
//! }
//!
//! fn main() -> Result<(), armature::Error> {
//!     App::new().panel(Hello::default()).run()
//! }
//! ```

use std::any::Any;

use futures::channel::oneshot;
use iced::keyboard::{Key, Modifiers, key::Physical};
use iced::{Element, Subscription, Task};

use crate::layout::{CustomInfo, Side};

/// One of the terminal's tabs (a window of Armature's own tmux server), as [`Notice::Tabs`] gives it.
pub use armature_core::monitor::LocalTab as Tab;

/// What Armature tells the panels. More kinds may come, so match with a `_` arm.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Notice {
    /// The terminal's tabs, left to right. Sent when they change (and once when the panel starts).
    Tabs(Vec<Tab>),
    /// The in-app browser's tabs, which one is in front, and whether the browser (rather than the
    /// terminal) is showing in the center.
    BrowserTabs {
        tabs: Vec<BrowserTab>,
        active: usize,
        showing: bool,
    },
    /// This panel got (`true`) or lost (`false`) the keyboard.
    Keys(bool),
    /// Close what you show in the center: Esc was pressed, or another screen took the center.
    Dismiss,
}

/// One of the in-app browser's tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserTab {
    pub title: String,
    pub url: String,
    /// Restored but not loaded yet; selecting it loads it.
    pub asleep: bool,
}

/// A key pressed while the panel holds the keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key: Key,
    pub physical: Physical,
    pub modifiers: Modifiers,
}

impl KeyPress {
    /// The letter on the key in a Latin layout, whatever input source is on (`j` on a Japanese
    /// keyboard is still `j`).
    #[must_use]
    pub fn latin(&self) -> Option<char> {
        self.key.to_latin(self.physical)
    }

    /// `c` with no modifier held.
    #[must_use]
    pub fn is(&self, c: char) -> bool {
        self.modifiers.is_empty() && self.latin() == Some(c)
    }
}

/// A panel of your own.
pub trait Panel: Send + 'static {
    /// The name in `panels.conf`: lowercase letters, digits and `-`. A key that is also a
    /// built-in panel's (`browser`, `music`, `sessions`, `clock`, `calendar`, `tasks`) replaces
    /// that panel in your build.
    const KEY: &'static str;
    /// Whether the panel takes the rest of its column's height, like the task list. By default
    /// it is as tall as its content.
    const FILLS: bool = false;
    /// The column the panel joins when it is turned on.
    const SIDE: Side = Side::Right;
    /// Whether the card leaves a margin around [`Panel::view`]. Turn it off to draw up to the
    /// panel's edges (a picture, a map).
    const PADDED: bool = true;
    /// Whether the panel is in the arrangement Armature starts with (before a `panels.conf` exists).
    const SHOWN: bool = true;
    /// Whether the panel takes the keyboard when it is clicked (or reached with ⌘ +
    /// [`Panel::SHORTCUT`]). Keys then go to [`Panel::key`] until the terminal takes them back.
    const KEYS: bool = false;
    /// The letter that, with ⌘, gives this panel the keyboard. Armature's own ⌘ keys come first.
    const SHORTCUT: Option<char> = None;

    /// The panel's own messages.
    type Message: Clone + std::fmt::Debug + Send + 'static;

    /// Draws the panel's content. Armature puts it on a card.
    fn view(&self) -> Element<'_, Self::Message>;

    /// Handles the panel's messages. `host` asks Armature for things (open a page, start Claude).
    fn update(&mut self, message: Self::Message, host: &mut Host) -> Task<Self::Message> {
        let _ = (message, host);
        Task::none()
    }

    /// Background work: timers, watchers. Runs only while the panel is shown.
    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::none()
    }

    /// Something Armature knows changed (see [`Notice`]).
    fn notice(&mut self, notice: &Notice, host: &mut Host) -> Task<Self::Message> {
        let _ = (notice, host);
        Task::none()
    }

    /// A key, while the panel holds the keyboard ([`Panel::KEYS`]). Return `None` to leave the
    /// key to Armature (its ⌘ keys, Esc), `Some(task)` when the panel used it.
    fn key(&mut self, key: &KeyPress, host: &mut Host) -> Option<Task<Self::Message>> {
        let _ = (key, host);
        None
    }

    /// What to show in the center of the window instead of the terminal, while there is
    /// something (a bigger calendar, a detail page). Armature sends [`Notice::Dismiss`] when the
    /// user closes it.
    fn center(&self) -> Option<Element<'_, Self::Message>> {
        None
    }

    /// What Claude should know about this panel: the files it reads, how to change what it
    /// shows. Armature passes the notes of the panels on screen to every Claude tab it starts (as
    /// part of the system prompt), so "add a task" or "put this in my panel" just works.
    /// Markdown, starting with a `##` heading. Use `tr!` for both languages.
    fn note(&self) -> Option<String> {
        None
    }

    /// Called once when Armature quits — the window's close button, ⌘Q, Quit in the Dock, and just
    /// before a restart — on every panel, shown or not. Armature ends the process right after, so
    /// `Drop` does not run: save what you need to keep here. It runs on the window's thread;
    /// keep it short.
    fn on_exit(&mut self) {}
}

/// What a panel can ask Armature to do, from [`Panel::update`].
///
/// Each request returns an [`Outcome`]. Drop it when you don't need to know, or turn it into one
/// of your messages with [`Outcome::map`] and return that task from `update`.
#[derive(Default)]
pub struct Host {
    pub(crate) requests: Vec<(Request, Reply)>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Host")
            .field(
                "requests",
                &self.requests.iter().map(|(request, _)| request).collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    OpenUrl(String),
    StartClaude { title: String, arguments: Vec<String> },
    SelectTab(String),
    MoveTabs(Vec<String>),
    SelectBrowserTab(usize),
    ReleaseKeys,
}

/// Where Armature puts the result of a request.
pub(crate) type Reply = oneshot::Sender<Result<(), String>>;

impl Host {
    /// Opens a web page (`http` or `https`) in the browser panel, or in the Mac's default browser
    /// when the browser panel is not shown. Anything else — `file:`, `javascript:`, a scheme
    /// that starts another app — is refused.
    pub fn open_url(&mut self, url: impl Into<String>) -> Outcome {
        self.ask(Request::OpenUrl(url.into()))
    }

    /// Opens a new Claude tab whose first message is `prompt`. The prompt is passed after `--`,
    /// so one that starts with `-` is still a message.
    pub fn open_claude(&mut self, prompt: impl Into<String>) -> Outcome {
        self.start_claude("claude", vec!["--".to_string(), prompt.into()])
    }

    /// Opens a new Claude tab named `title`. `arguments` are passed to `claude` as they are —
    /// options first, the first message last. Put `--` before a message that may start with `-`.
    pub fn start_claude(&mut self, title: impl Into<String>, arguments: Vec<String>) -> Outcome {
        self.ask(Request::StartClaude {
            title: title.into(),
            arguments,
        })
    }

    /// Brings a terminal tab to the front (its [`Tab::target`]).
    pub fn select_tab(&mut self, target: impl Into<String>) -> Outcome {
        self.ask(Request::SelectTab(target.into()))
    }

    /// Puts the terminal's tabs in this order (their [`Tab::target`]s, left to right). Tabs
    /// not listed keep their places after the listed ones. A new [`Notice::Tabs`] follows.
    pub fn move_tabs(&mut self, order: Vec<String>) -> Outcome {
        self.ask(Request::MoveTabs(order))
    }

    /// Shows the browser's tab at `index` in the center.
    pub fn select_browser_tab(&mut self, index: usize) -> Outcome {
        self.ask(Request::SelectBrowserTab(index))
    }

    /// Gives the keyboard back to the terminal.
    pub fn release_keys(&mut self) -> Outcome {
        self.ask(Request::ReleaseKeys)
    }

    fn ask(&mut self, request: Request) -> Outcome {
        let (reply, answer) = oneshot::channel();
        self.requests.push((request, reply));
        Outcome(answer)
    }
}

/// How a request to [`Host`] turned out: `Ok(())` when Armature did it, `Err` with the reason (in
/// the user's language) when it could not or would not.
#[derive(Debug)]
pub struct Outcome(oneshot::Receiver<Result<(), String>>);

impl Outcome {
    /// Turns the outcome into one of your panel's messages. Return the task from
    /// [`Panel::update`].
    pub fn map<M: Send + 'static>(
        self,
        f: impl FnOnce(Result<(), String>) -> M + Send + 'static,
    ) -> Task<M> {
        Task::perform(self.0, move |answer| {
            f(answer.unwrap_or_else(|_| Err("Armature dropped the request".to_string())))
        })
    }
}

/// A panel's message with its type hidden, so every custom panel fits in one app message.
///
/// Only `Send` is asked of the message (not `Sync`): each payload is cloned, never shared.
pub struct Payload(Box<dyn Carried>);

trait Carried: Any + Send {
    fn clone_box(&self) -> Box<dyn Carried>;
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Send + Clone> Carried for T {
    fn clone_box(&self) -> Box<dyn Carried> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Clone for Payload {
    fn clone(&self) -> Self {
        Self((*self.0).clone_box())
    }
}

impl std::fmt::Debug for Payload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Payload")
    }
}

impl Payload {
    fn new<T: Any + Send + Clone>(value: T) -> Self {
        Self(Box::new(value))
    }

    fn get<T: Clone + 'static>(&self) -> Option<T> {
        (*self.0).as_any().downcast_ref::<T>().cloned()
    }
}

/// The face of a [`Panel`] the app works with, whatever its message type.
pub(crate) trait DynPanel: Send {
    fn view(&self) -> Element<'_, Payload>;
    fn update(&mut self, payload: Payload, host: &mut Host) -> Task<Payload>;
    fn subscription(&self) -> Subscription<Payload>;
    fn notice(&mut self, notice: &Notice, host: &mut Host) -> Task<Payload>;
    fn key(&mut self, key: &KeyPress, host: &mut Host) -> Option<Task<Payload>>;
    fn center(&self) -> Option<Element<'_, Payload>>;
    fn note(&self) -> Option<String>;
    fn on_exit(&mut self);
}

impl<P: Panel> DynPanel for P {
    fn view(&self) -> Element<'_, Payload> {
        Panel::view(self).map(Payload::new::<P::Message>)
    }

    fn update(&mut self, payload: Payload, host: &mut Host) -> Task<Payload> {
        match payload.get::<P::Message>() {
            Some(message) => Panel::update(self, message, host).map(Payload::new::<P::Message>),
            None => Task::none(),
        }
    }

    fn subscription(&self) -> Subscription<Payload> {
        Panel::subscription(self).map(Payload::new::<P::Message>)
    }

    fn notice(&mut self, notice: &Notice, host: &mut Host) -> Task<Payload> {
        Panel::notice(self, notice, host).map(Payload::new::<P::Message>)
    }

    fn key(&mut self, key: &KeyPress, host: &mut Host) -> Option<Task<Payload>> {
        Panel::key(self, key, host).map(|task| task.map(Payload::new::<P::Message>))
    }

    fn center(&self) -> Option<Element<'_, Payload>> {
        Panel::center(self).map(|view| view.map(Payload::new::<P::Message>))
    }

    fn note(&self) -> Option<String> {
        Panel::note(self)
    }

    fn on_exit(&mut self) {
        Panel::on_exit(self);
    }
}

/// What the layout needs to know about a panel type.
pub(crate) fn info<P: Panel>() -> CustomInfo {
    CustomInfo {
        key: P::KEY,
        fills: P::FILLS,
        side: P::SIDE,
        padded: P::PADDED,
        shown: P::SHOWN,
        keys: P::KEYS,
        shortcut: P::SHORTCUT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Counter {
        count: u32,
    }

    #[derive(Clone, Debug)]
    enum Bump {
        Up,
        Open,
    }

    impl Panel for Counter {
        const KEY: &'static str = "counter";
        type Message = Bump;

        fn view(&self) -> Element<'_, Bump> {
            iced::widget::text(self.count).into()
        }

        fn update(&mut self, message: Bump, host: &mut Host) -> Task<Bump> {
            match message {
                Bump::Up => self.count += 1,
                Bump::Open => {
                    host.open_url("https://example.com");
                }
            }
            Task::none()
        }
    }

    #[test]
    fn a_payload_reaches_its_own_panel_and_no_other() {
        let mut counter = Counter::default();
        let mut host = Host::default();
        let _ = DynPanel::update(&mut counter, Payload::new(Bump::Up), &mut host);
        assert_eq!(counter.count, 1);
        let _ = DynPanel::update(&mut counter, Payload::new("not a Bump"), &mut host);
        assert_eq!(counter.count, 1, "a foreign message is dropped");
        let _ = DynPanel::update(&mut counter, Payload::new(Bump::Open), &mut host);
        let requests: Vec<Request> = host.requests.into_iter().map(|(request, _)| request).collect();
        assert_eq!(requests, vec![Request::OpenUrl("https://example.com".to_string())]);
    }

    #[test]
    fn a_prompt_that_starts_with_a_dash_is_still_the_first_message() {
        let mut host = Host::default();
        let _ = host.open_claude("-v means verbose, explain");
        let requests: Vec<Request> = host.requests.into_iter().map(|(request, _)| request).collect();
        assert_eq!(
            requests,
            vec![Request::StartClaude {
                title: "claude".to_string(),
                arguments: vec!["--".to_string(), "-v means verbose, explain".to_string()],
            }]
        );
    }

    #[test]
    fn the_outcome_of_a_request_comes_back_to_the_panel() {
        let mut host = Host::default();
        let outcome = host.open_url("file:///etc/passwd");
        let (_, reply) = host.requests.pop().expect("a request");
        reply.send(Err("refused".to_string())).unwrap();
        let mut answer = outcome.0;
        assert_eq!(
            answer.try_recv().unwrap(),
            Some(Err("refused".to_string()))
        );
        // 捨てた Outcome に返しても何も起きない(聞き手がいないだけ)。
        let _ = host.open_url("https://example.com");
        let (_, reply) = host.requests.pop().expect("a request");
        assert!(reply.send(Ok(())).is_err());
    }

    #[test]
    fn a_message_needs_only_send_and_clone() {
        // Cell は Send だが Sync ではない。それでもパネルの合図に使える。
        #[derive(Clone, Debug)]
        struct NotSync(std::cell::Cell<u8>);
        let payload = Payload::new(NotSync(std::cell::Cell::new(3)));
        let copy = payload.clone();
        assert_eq!(copy.get::<NotSync>().map(|value| value.0.get()), Some(3));
        assert!(payload.get::<u8>().is_none());
    }

    #[test]
    fn on_exit_reaches_the_panel() {
        #[derive(Default)]
        struct Saver {
            saved: bool,
        }
        impl Panel for Saver {
            const KEY: &'static str = "saver";
            type Message = ();
            fn view(&self) -> Element<'_, ()> {
                iced::widget::text("").into()
            }
            fn on_exit(&mut self) {
                self.saved = true;
            }
        }
        let mut panel = Saver::default();
        DynPanel::on_exit(&mut panel);
        assert!(panel.saved);
        assert!(info::<Saver>().padded, "a card keeps its margin unless the panel turns it off");
    }

    #[test]
    fn the_layout_learns_the_panel_from_its_type() {
        let info = info::<Counter>();
        assert_eq!(info.key, "counter");
        assert!(!info.fills);
        assert_eq!(info.side, Side::Right);
    }
}
