//! The clock: the time, large, in Helvetica Neue Bold, glowing. The date is the calendar's job.

use std::time::Duration;

use iced::widget::{container, responsive, text};
use iced::{Length, Subscription, Task};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::{Element, Host, Panel, font, glow, palette};

/// The panel's height: the digits (56 px × line height 1.3) and the padding above and below.
const ROW_HEIGHT: f32 = 85.0;
/// The digits at full size. They are the point of the panel, so they take the width they can.
const DIGIT_SIZE: f32 = 56.0;
/// The smallest the digits get in a narrow window; below this they can't be read.
const DIGIT_MIN: f32 = 15.0;
/// Width of one digit in ems (measured). Text can't be measured before it is laid out, so the
/// size is worked out from this first.
const ADVANCE: f32 = 0.6;
const PADDING_X: f32 = 8.0;
const PADDING_Y: f32 = 6.0;

/// The clock panel (`clock` in `panels.conf`).
pub struct Clock {
    /// `HH:MM`.
    time: String,
}

#[derive(Clone, Debug)]
pub enum Message {
    Tick,
}

impl Clock {
    #[must_use]
    pub fn new() -> Self {
        Self { time: now() }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Panel for Clock {
    const KEY: &'static str = "clock";
    const PADDED: bool = false;
    type Message = Message;

    fn view(&self) -> Element<'_, Message> {
        let time = self.time.as_str();
        container(responsive(move |size| {
            let digit = digit_size(size.width);
            let face = glow::glow(
                move |ink| {
                    text(time)
                        .font(font::CLOCK_HELVETICA)
                        .size(digit)
                        .color(ink)
                        .wrapping(text::Wrapping::None)
                        .into()
                },
                palette::text_lit(),
                1.0,
            );
            container(face).center(Length::Fill).into()
        }))
        .padding([PADDING_Y as u16, PADDING_X as u16])
        .width(Length::Fill)
        .height(Length::Fixed(ROW_HEIGHT))
        .into()
    }

    fn update(&mut self, message: Message, _host: &mut Host) -> Task<Message> {
        match message {
            Message::Tick => self.time = now(),
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    }
}

fn now() -> String {
    let now = Timestamp::now().to_zoned(TimeZone::system());
    label(now.hour(), now.minute())
}

fn label(hour: i8, minute: i8) -> String {
    format!("{hour:02}:{minute:02}")
}

/// The digit size for a width. **Windows shrink**: at a fixed size a narrow panel wrapped the
/// time onto two lines.
fn digit_size(available: f32) -> f32 {
    (available / group_ratio()).clamp(DIGIT_MIN, DIGIT_SIZE)
}

/// How many digit sizes wide `HH:MM` is, plus one digit of room so it doesn't touch the edges.
fn group_ratio() -> f32 {
    5.0f32.mul_add(ADVANCE, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_uses_two_digit_padding() {
        assert_eq!(label(8, 5), "08:05");
        assert_eq!(label(19, 1), "19:01");
    }

    /// Narrow the window and the digits shrink instead of wrapping.
    #[test]
    fn the_clock_shrinks_with_the_window_instead_of_wrapping() {
        for available in [150.0_f32, 200.0, 260.0, 330.0, 401.0, 520.0] {
            let digit = digit_size(available);
            let need = digit * group_ratio();
            assert!(
                need <= available + 0.01 || digit <= DIGIT_MIN,
                "width {available}: {need} does not fit"
            );
        }
        assert!((digit_size(1000.0) - DIGIT_SIZE).abs() < 0.01);
        assert!(digit_size(120.0) < DIGIT_SIZE);
        assert!((digit_size(40.0) - DIGIT_MIN).abs() < 0.01);
        assert!(digit_size(200.0) <= digit_size(300.0));
        assert!(digit_size(300.0) <= digit_size(400.0));
    }
}
