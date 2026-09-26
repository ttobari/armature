//! Apple Music: the song playing now, and back / play / next, volume, shuffle, repeat and the
//! playlists. Music.app is driven with AppleScript ([`apple`]); the first time, macOS asks
//! whether Armature may control it.
//!
//! **The watch never wakes Music.** It checks that Music is running and leaves it alone
//! otherwise. Music is started only when the user presses ▶ or opens the playlists.

mod apple;

use std::time::Duration;

use iced::widget::{button, column, container, row, scrollable, slider, text};
use iced::{Alignment, Background, Border, Length, Subscription, Task};

use crate::palette::{
    self, accent_active, surface_active, surface_raised, text_faint, text_muted, text_primary,
    text_secondary,
};
use crate::{Element, Host, Panel, ellipsized};

use apple::{Command_, Now, State};

/// The cover's size.
const COVER: f32 = 38.0;
/// The volume moves in steps of 5: 1 is too fine, 10 too coarse.
const VOLUME_STEP: u8 = 5;
/// The volume bar's height. It shares a row with the buttons, so it is thin.
const VOLUME_BAR: f32 = 12.0;
/// Room between the speaker mark and the bar: touching, they read as one part.
const VOLUME_GAP: f32 = 7.0;
const LIST_HEIGHT: f32 = 96.0;

mod icon {
    pub const PLAY: &str = "\u{f04b}";
    pub const PAUSE: &str = "\u{f04c}";
    pub const BACKWARD: &str = "\u{f048}";
    pub const FORWARD: &str = "\u{f051}";
    pub const SHUFFLE: &str = "\u{f074}";
    /// Repeat (the Material Design loop). Repeating one song swaps the whole glyph for the loop
    /// with a 1 in it: a tiny 1 on the loop's shoulder could not be read.
    pub const REPEAT: &str = "\u{f0456}";
    pub const REPEAT_ONE: &str = "\u{f0458}";
    pub const LIST: &str = "\u{f03a}";
    pub const VOLUME: &str = "\u{f028}";
    pub const VOLUME_OFF: &str = "\u{f026}";
}

/// The music panel (`music` in `panels.conf`).
#[derive(Default)]
pub struct Music {
    now: Now,
    artwork: Option<iced::widget::image::Handle>,
    /// The volume while the slider is held. **Read before `now`**: Music is read again only
    /// every few seconds, and the knob would jump back under the finger meanwhile.
    volume: Option<u8>,
    /// Which song the artwork is for; it is fetched again only when the song changes.
    song: String,
    playlists_open: bool,
    playlists: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Message {
    Tick,
    Loaded(Box<Now>),
    Artwork(Option<std::path::PathBuf>),
    Send(Command_),
    Playlists,
    PlaylistsLoaded(Vec<String>),
    Play(String),
    Volume(u8),
}

impl Music {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Panel for Music {
    const KEY: &'static str = "music";
    const SIDE: crate::Side = crate::Side::Left;
    const PADDED: bool = false;
    /// Not in the starting arrangement: it asks macOS for permission to control Music.
    const SHOWN: bool = false;
    type Message = Message;

    fn view(&self) -> Element<'_, Message> {
        let now = &self.now;
        let cover: Element<'_, Message> = match self.artwork.as_ref() {
            // A transparent cover needs something under it, or the panel shows through.
            Some(handle) => container(
                iced::widget::image(handle.clone())
                    .width(Length::Fixed(COVER))
                    .height(Length::Fixed(COVER))
                    .content_fit(iced::ContentFit::Cover),
            )
            .style(|_| container::Style {
                background: Some(Background::Color(iced::Color::WHITE)),
                border: Border {
                    radius: palette::radius_control().into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .clip(true)
            .into(),
            None => container(text("♪").size(16).color(text_faint()))
                .center(Length::Fixed(COVER))
                .into(),
        };

        // Nothing is written while stopped: the row of buttons says it already.
        let title = if now.state.has_track() { now.name.as_str() } else { "" };
        let facts = column![
            ellipsized::text(title, 12.0, text_primary()),
            ellipsized::text(&now.artist, 10.0, text_muted()),
        ]
        .spacing(1)
        .width(Length::Fill);

        let key = |label: &'static str, message: Message, lit: bool| {
            button(
                text(label)
                    .size(11)
                    .color(if lit { accent_active() } else { text_muted() }),
            )
            .padding([1, 4])
            .on_press(message)
            .style(|_, status| key_style(status))
        };
        let playing = now.state == State::Playing;
        let next_repeat = match now.repeat.as_str() {
            "off" => "all",
            "all" => "one",
            _ => "off",
        };
        let controls = row![
            key(icon::BACKWARD, Message::Send(Command_::Previous), false),
            key(
                if playing { icon::PAUSE } else { icon::PLAY },
                Message::Send(Command_::PlayPause),
                false
            ),
            key(icon::FORWARD, Message::Send(Command_::Next), false),
            // The volume sits in the middle of the same row, so the panel stays two rows tall.
            self.volume_bar(),
            key(
                icon::SHUFFLE,
                Message::Send(Command_::Shuffle(!now.shuffle)),
                now.shuffle
            ),
            repeat_key(&now.repeat, next_repeat),
            key(icon::LIST, Message::Playlists, self.playlists_open),
        ]
        .spacing(2)
        .align_y(Alignment::Center);

        let mut body = column![
            row![cover, facts].spacing(8).align_y(Alignment::Center),
            controls,
        ]
        .spacing(4)
        .width(Length::Fill);

        if self.playlists_open {
            let mut list = column![].spacing(1).width(Length::Fill);
            for name in &self.playlists {
                list = list.push(
                    button(ellipsized::text(name, 10.0, text_secondary()))
                        .padding([2, 5])
                        .width(Length::Fill)
                        .on_press(Message::Play(name.clone()))
                        .style(|_, status| button::Style {
                            background: matches!(
                                status,
                                button::Status::Hovered | button::Status::Pressed
                            )
                            .then_some(Background::Color(surface_active())),
                            text_color: text_secondary(),
                            border: Border {
                                radius: palette::radius_control().into(),
                                ..Border::default()
                            },
                            ..button::Style::default()
                        }),
                );
            }
            body = body.push(
                scrollable(list)
                    .direction(scrollable::Direction::Vertical(
                        scrollable::Scrollbar::hidden(),
                    ))
                    .width(Length::Fill)
                    .height(Length::Fixed(LIST_HEIGHT)),
            );
        }

        container(body)
            .padding([6, 8])
            .width(Length::Fill)
            .height(Length::Shrink)
            .into()
    }

    fn update(&mut self, message: Message, _host: &mut Host) -> Task<Message> {
        match message {
            Message::Tick => Task::perform(
                async {
                    tokio::task::spawn_blocking(apple::snapshot)
                        .await
                        .unwrap_or_default()
                },
                |now| Message::Loaded(Box::new(now)),
            ),
            Message::Loaded(now) => {
                let now = *now;
                let song = now.key();
                let changed = song != self.song;
                // The value we sent has come back: let go of the held one.
                if self.volume == Some(now.volume) {
                    self.volume = None;
                }
                self.now = now;
                if !changed {
                    return Task::none();
                }
                self.song = song.clone();
                self.artwork = None;
                if !self.now.state.has_track() {
                    return Task::none();
                }
                // The cover only when the song changes: no reason to write a JPEG every 5 s.
                let target = artwork_path(&song);
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || apple::artwork(&target))
                            .await
                            .unwrap_or_default()
                    },
                    Message::Artwork,
                )
            }
            Message::Artwork(path) => {
                // Read by content, not by extension: Music sometimes writes a PNG as `.jpg`.
                self.artwork = path
                    .and_then(|path| std::fs::read(path).ok())
                    .map(iced::widget::image::Handle::from_bytes);
                Task::none()
            }
            Message::Send(command) => {
                // ▶ while Music is asleep wakes it first, or pressing ▶ would do nothing.
                let wake = self.now.state == State::Off && command == Command_::PlayPause;
                Task::perform(
                    async move {
                        let _ = tokio::task::spawn_blocking(move || {
                            if wake {
                                apple::launch();
                            }
                            apple::send(&command)
                        })
                        .await;
                    },
                    |()| Message::Tick,
                )
            }
            Message::Playlists => {
                self.playlists_open = !self.playlists_open;
                if !self.playlists_open {
                    return Task::none();
                }
                // A sleeping Music has no playlists: wake it, then ask.
                let wake = self.now.state == State::Off;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            if wake {
                                apple::launch();
                            }
                            apple::playlists()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    Message::PlaylistsLoaded,
                )
            }
            Message::PlaylistsLoaded(names) => {
                // An empty answer (Music was asleep) does not wipe the list.
                if !names.is_empty() {
                    self.playlists = names;
                }
                Task::none()
            }
            Message::Volume(level) => {
                self.volume = Some(level);
                Task::perform(
                    async move {
                        let _ = tokio::task::spawn_blocking(move || {
                            apple::send(&Command_::Volume(level))
                        })
                        .await;
                    },
                    |()| Message::Tick,
                )
            }
            Message::Play(name) => {
                self.playlists_open = false;
                Task::perform(
                    async move {
                        let _ = tokio::task::spawn_blocking(move || {
                            // Playlists are for shuffling.
                            let _ = apple::send(&Command_::Shuffle(true));
                            apple::send(&Command_::Play(name))
                        })
                        .await;
                    },
                    |()| Message::Tick,
                )
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // Every 5 s while a song is on, every 20 s otherwise: one look holds Music for 0.17 s.
        let every = if self.now.state.has_track() { 5 } else { 20 };
        iced::time::every(Duration::from_secs(every)).map(|_| Message::Tick)
    }
}

impl Music {
    /// Music.app's own volume (not the Mac's). No number: the bar's length says it.
    fn volume_bar(&self) -> Element<'_, Message> {
        let level = self.volume.unwrap_or(self.now.volume).min(100);
        let mark = if level == 0 { icon::VOLUME_OFF } else { icon::VOLUME };
        let bar = slider(0_u8..=100, level, Message::Volume)
            .step(VOLUME_STEP)
            .width(Length::Fill)
            .height(VOLUME_BAR)
            .style(|_, status| {
                use iced::widget::slider::{Handle, HandleShape, Rail, Status, Style};
                let dragging = status == Status::Dragged;
                Style {
                    rail: Rail {
                        backgrounds: (
                            Background::Color(accent_active()),
                            Background::Color(surface_raised()),
                        ),
                        width: if dragging { 6.0 } else { 3.0 },
                        border: Border {
                            radius: palette::radius_control().into(),
                            ..Border::default()
                        },
                    },
                    handle: Handle {
                        shape: HandleShape::Rectangle {
                            width: if dragging { 3 } else { 0 },
                            border_radius: 0.0.into(),
                        },
                        background: Background::Color(if dragging {
                            accent_active()
                        } else {
                            iced::Color::TRANSPARENT
                        }),
                        border_width: 0.0,
                        border_color: iced::Color::TRANSPARENT,
                    },
                }
            });
        container(
            row![text(mark).size(9).color(text_muted()), bar]
                .spacing(VOLUME_GAP)
                .align_y(Alignment::Center)
                .width(Length::Fill),
        )
        .padding([0, 8])
        .width(Length::Fill)
        .into()
    }
}

/// The repeat button: off → all → one → off.
fn repeat_key(repeat: &str, next: &'static str) -> Element<'static, Message> {
    let glyph = if repeat == "one" { icon::REPEAT_ONE } else { icon::REPEAT };
    let colour = if repeat == "off" { text_muted() } else { accent_active() };
    button(text(glyph).size(12).color(colour))
        .padding([1, 4])
        .on_press(Message::Send(Command_::Repeat(next.to_string())))
        .style(|_, status| key_style(status))
        .into()
}

fn key_style(status: button::Status) -> button::Style {
    button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then_some(Background::Color(surface_raised())),
        text_color: text_muted(),
        border: Border {
            radius: palette::radius_control().into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Where the cover is written. A name per song: with one name, iced keeps showing the first
/// picture after the song changes.
fn artwork_path(key: &str) -> std::path::PathBuf {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    armature_core::paths::state().join(format!("artwork-{hash:016x}.jpg"))
}
