//! 設定の画面——表示の言語と配色。
//!
//! 中央に出る(ヘルプと同じ場所)。**触った瞬間に窓ぜんぶへ効く**——「適用」の札は無い。
//! 言語は `crate::locale`、配色は `crate::appearance` に残り、次に窓を起こしたときも同じ姿で立つ。
//! パネルの並びは設定に置かない(`panels.conf` を Claude に頼んで書き換える)。
//!
//! 操作は2口。
//!
//! - マウス —— 値を直に押す
//! - キー —— `L` / `H` で言語、`J` / `K` で配色を回す

use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Background, Border, Element, Length};

use crate::font;
use crate::palette::{self, Theme};
use armature_core::lang::{self, Lang};
use armature_core::tr;

/// 設定画面から出る合図。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// 表示の言語を [`Lang::ALL`] の何番目かへ。
    Language(usize),
    /// 配色を [`Theme::ALL`] の何番目かへ。
    Theme(usize),
}

/// 配色を1つ進める / 戻す合図。
#[must_use]
pub fn turn_theme(forward: bool) -> Message {
    let count = Theme::ALL.len();
    let current = Theme::ALL
        .iter()
        .position(|theme| *theme == palette::theme())
        .unwrap_or(0);
    let next = if forward {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    };
    Message::Theme(next)
}

/// 言語を1つ進める / 戻す合図。
#[must_use]
pub fn turn_language(forward: bool) -> Message {
    let count = Lang::ALL.len();
    let current = lang::current().index();
    let next = if forward {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    };
    Message::Language(next)
}

pub fn view() -> Element<'static, Message> {
    let body = column![language_row(), theme_row()]
        .spacing(22)
        // 右端に少し逃がす。詰めきると、いちばん右のチップが縦の巻取りの下へ
        // 潜って字が切れる。
        .padding(iced::Padding::default().right(12))
        .width(Length::Fill);

    container(scrollable(body).height(Length::Fill).width(Length::Fill))
        .padding([40, 48])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| palette::card_style())
        .into()
}

/// 節の見出し。
fn heading(label: &str) -> Element<'static, Message> {
    text(label.to_string())
        .size(11)
        .font(font::UI)
        .color(palette::text_faint())
        .into()
}

/// 言語の行。
fn language_row() -> Element<'static, Message> {
    let current = lang::current().index();
    let mut chips = row![].spacing(6);
    for (index, language) in Lang::ALL.iter().enumerate() {
        chips = chips.push(chip(language.label(), index == current, Message::Language(index)));
    }
    column![heading("Language"), chips.wrap()]
    .spacing(10)
    .width(Length::Fill)
    .into()
}

/// 配色の行。触った瞬間に窓ぜんぶと端末の色が入れ替わる。
fn theme_row() -> Element<'static, Message> {
    let current = palette::theme();
    let mut chips = column![].spacing(2);
    for (index, theme) in Theme::ALL.iter().enumerate() {
        chips = chips.push(chip(theme.label(), *theme == current, Message::Theme(index)));
    }
    column![heading(tr!("Theme", "配色")), chips]
        .spacing(10)
        .width(Length::Fill)
        .into()
}

/// 選べるものの1つ。選んでいるものは字の明るさと太さだけで示す。
fn chip(label: &'static str, selected: bool, message: Message) -> Element<'static, Message> {
    let (color, face) = if selected {
        (palette::text_lit(), font::UI_STRONG)
    } else {
        (palette::text_faint(), font::UI)
    };
    button(text(label).size(12).font(face).color(color))
        .padding([3, 8])
        .on_press(message)
        .style(|_, status| chip_style(status))
        .into()
}

/// 押し口の面。指を乗せたときだけ浮く。
fn chip_style(status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then_some(Background::Color(palette::surface_raised())),
        text_color: palette::text_primary(),
        border: Border {
            radius: palette::radius_control().into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_the_language_wraps_at_both_ends() {
        let _en = armature_core::lang::scoped(Lang::En);
        let count = Lang::ALL.len();
        let current = Lang::En.index();
        assert_eq!(turn_language(true), Message::Language((current + 1) % count));
        assert_eq!(
            turn_language(false),
            Message::Language((current + count - 1) % count)
        );
    }
}
