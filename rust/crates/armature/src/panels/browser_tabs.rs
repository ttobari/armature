//! The in-app browser's tabs, one line each. Click one to show it in the center. This panel is
//! also the browser's switch: while it is shown, pages open inside the window; without it,
//! they open in the Mac's default browser.

use iced::widget::{Space, button, column, container, responsive, row, scrollable, text};
use iced::{Alignment, Background, Border, Length, Task};

use crate::palette::{self, surface_raised, text_faint, text_muted, text_primary, text_secondary};
use crate::{BrowserTab, Element, Host, Notice, Panel, ellipsized, font};

/// One line's height with the gap: 11 px text (about 15 px of line) + 2 px padding + 1 px gap.
/// **It must match what is drawn**, or the count of lines that fit is off and a cut-off line shows.
const ROW: f32 = 20.0;
const GAP: f32 = 1.0;
/// The "more above / below" marks.
const HINT: f32 = 14.0;
/// Still more lines below / above.
const MORE_DOWN: &str = "\u{f078}";
const MORE_UP: &str = "\u{f077}";

/// The browser tab list (`browser` in `panels.conf`).
pub struct BrowserTabs {
    tabs: Vec<BrowserTab>,
    active: usize,
    /// Whether the browser (not the terminal) is in the center.
    showing: bool,
    list: iced::widget::Id,
    scroll: Option<scrollable::Viewport>,
}

#[derive(Clone, Debug)]
pub enum Message {
    Pressed(usize),
    Scrolled(scrollable::Viewport),
}

impl BrowserTabs {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            showing: false,
            list: iced::widget::Id::unique(),
            scroll: None,
        }
    }

    /// Scrolls so the tab in front is in view. Placing it at `active / (count − 1)` of the way
    /// down puts the first at the top and the last at the bottom, and keeps any one inside the
    /// window without knowing how many lines fit.
    fn follow(&self) -> Task<Message> {
        let count = self.tabs.len();
        if count < 2 {
            return Task::none();
        }
        #[allow(clippy::cast_precision_loss)]
        let offset = (self.active as f32 / (count - 1) as f32).clamp(0.0, 1.0);
        iced::widget::operation::snap_to(
            self.list.clone(),
            iced::advanced::widget::operation::scrollable::RelativeOffset { x: 0.0, y: offset },
        )
    }
}

impl Default for BrowserTabs {
    fn default() -> Self {
        Self::new()
    }
}

impl Panel for BrowserTabs {
    const KEY: &'static str = "browser";
    const SIDE: crate::Side = crate::Side::Left;
    const FILLS: bool = true;
    const PADDED: bool = false;
    type Message = Message;

    fn view(&self) -> Element<'_, Message> {
        if self.tabs.is_empty() {
            return Space::new().height(Length::Fill).into();
        }
        // **Cut the list at a whole number of lines.** A half line doesn't read as "there's
        // more", only as broken. Scrolling is by the wheel; no scroll bar.
        let count = self.tabs.len();
        let scroll = self.scroll;
        let list = responsive(move |size| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let full_rows = (size.height / ROW).floor().max(1.0) as usize;
            let overflow = count > full_rows;
            let room = if overflow {
                (size.height - HINT * 2.0 - 2.0).max(ROW)
            } else {
                size.height
            };
            let rows = (room / ROW).floor().max(1.0);
            let mut lines = column![].spacing(GAP);
            for (index, tab) in self.tabs.iter().enumerate() {
                lines = lines.push(tab_view(tab, index, self.showing && index == self.active));
            }
            let window = scrollable(lines)
                .id(self.list.clone())
                .on_scroll(Message::Scrolled)
                .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::hidden()))
                .width(Length::Fill)
                .height(Length::Fixed(rows * ROW));
            // Marks only when it doesn't all fit, and only on the side where lines are hidden.
            if !overflow {
                return window.into();
            }
            let (above, below) = match scroll {
                Some(view) => {
                    let span = (view.content_bounds().height - view.bounds().height).max(0.0);
                    let offset = view.absolute_offset().y;
                    (offset > 0.5, offset < span - 0.5)
                }
                None => (false, true),
            };
            let band = |show: bool, glyph: &'static str| {
                let mark: Element<'static, Message> = if show {
                    text(glyph).size(9).color(text_faint()).into()
                } else {
                    Space::new().into()
                };
                container(mark)
                    .center_x(Length::Fill)
                    .center_y(Length::Fixed(HINT))
            };
            column![band(above, MORE_UP), window, band(below, MORE_DOWN)]
                .spacing(1)
                .width(Length::Fill)
                .into()
        });
        container(list)
            .padding([7, 9])
            .width(Length::Fill)
            .height(Length::Fill)
            .clip(true)
            .into()
    }

    fn update(&mut self, message: Message, host: &mut Host) -> Task<Message> {
        match message {
            Message::Pressed(index) => {
                host.select_browser_tab(index);
            }
            Message::Scrolled(viewport) => self.scroll = Some(viewport),
        }
        Task::none()
    }

    fn notice(&mut self, notice: &Notice, _host: &mut Host) -> Task<Message> {
        let Notice::BrowserTabs { tabs, active, showing } = notice else {
            return Task::none();
        };
        let moved = *active != self.active || tabs.len() != self.tabs.len();
        self.tabs.clone_from(tabs);
        self.active = *active;
        self.showing = *showing;
        if moved { self.follow() } else { Task::none() }
    }
}

/// The line's text: the page's title, or its address while it has none. Cut to the width later,
/// by the widget that knows the width.
fn label(tab: &BrowserTab) -> &str {
    if tab.title.trim().is_empty() { tab.url.as_str() } else { tab.title.as_str() }
}

fn tab_view(tab: &BrowserTab, index: usize, active: bool) -> Element<'_, Message> {
    let label = label(tab);
    // A restored tab that hasn't loaded yet is dimmed; selecting it loads it.
    let ink = if active {
        palette::text_lit()
    } else if tab.asleep {
        text_muted()
    } else {
        text_secondary()
    };
    let content = row![
        ellipsized::styled(label, 11.0, ink, if active { font::UI_STRONG } else { font::UI }),
        Space::new().width(Length::Fixed(4.0))
    ]
    .spacing(5)
    .align_y(Alignment::Center);
    button(content)
        .padding([2, 5])
        .width(Length::Fill)
        .on_press(Message::Pressed(index))
        .style(move |_, status| {
            // The tab in front is shown by its text alone; a face only under the pointer.
            let background = matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then_some(Background::Color(surface_raised()));
            button::Style {
                background,
                text_color: text_primary(),
                border: Border {
                    radius: palette::radius_control().into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_keeps_the_whole_title_and_falls_back_to_the_address() {
        let titled = BrowserTab {
            title: "A title which is much too long for the browser card".into(),
            url: "https://example.com/a/very/long/path".into(),
            asleep: false,
        };
        assert_eq!(label(&titled), titled.title);
        let blank = BrowserTab {
            title: "  ".into(),
            url: "日本語の長いURL文字列".into(),
            asleep: false,
        };
        assert_eq!(label(&blank), blank.url);
    }
}
