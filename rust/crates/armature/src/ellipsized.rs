//! 割り当てられた実ピクセル幅で末尾を省略する1行テキスト。
//!
//! Iced 0.14の標準`Text`はnowrapとclipは持つが、末尾`…`は持たない。
//! レイアウト後の残り幅を受け取って実フォントで測るため、隣のmetadataと重ならない。

use iced::advanced::text::{self as advanced_text, paragraph::Paragraph as _};
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Layout, Widget, layout, mouse, renderer};
use iced::{Color, Element, Length, Pixels, Size, Theme};

/// 1行を置く。書体は窓の既定。
pub fn text<'a, Message: 'a>(content: &'a str, size: f32, color: Color) -> Element<'a, Message> {
    Element::new(EllipsizedText {
        content,
        size: Pixels(size),
        color,
        font: None,
    })
}

/// 書体を指定して置く。太字の見出しを1行で省略したいとき(タスクの束の見出し)に使う。
pub fn styled<'a, Message: 'a>(
    content: &'a str,
    size: f32,
    color: Color,
    font: iced::Font,
) -> Element<'a, Message> {
    Element::new(EllipsizedText {
        content,
        size: Pixels(size),
        color,
        font: Some(font),
    })
}

struct EllipsizedText<'a> {
    content: &'a str,
    size: Pixels,
    color: Color,
    /// 指定が無ければ描く側の既定の書体。太字を指定すると字幅が変わるので、
    /// 省略位置の目安として持ち越す(`State` の作り直しの合図にもなる)。
    font: Option<iced::Font>,
}

type TextParagraph = <iced::Renderer as advanced_text::Renderer>::Paragraph;

#[derive(Default)]
struct State {
    paragraph: advanced_text::paragraph::Plain<TextParagraph>,
    source: String,
    width: f32,
    size: f32,
    font: Option<iced::Font>,
}

impl<Message> Widget<Message, Theme, iced::Renderer> for EllipsizedText<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::sized(limits, Length::Fill, Length::Shrink, |limits| {
            let bounds = limits.max();
            let font = self
                .font
                .unwrap_or_else(|| advanced_text::Renderer::default_font(renderer));
            let state = tree.state.downcast_mut::<State>();
            if state.source != self.content
                || state.width != bounds.width
                || state.size != self.size.0
                || state.font != self.font
            {
                let visible = ellipsize_to_width(self.content, bounds.width, |candidate| {
                    let paragraph = TextParagraph::with_text(advanced_text::Text {
                        content: candidate,
                        bounds: Size::new(bounds.width.max(1.0), 100.0),
                        size: self.size,
                        line_height: advanced_text::LineHeight::default(),
                        font,
                        align_x: advanced_text::Alignment::Default,
                        align_y: iced::alignment::Vertical::Top,
                        shaping: advanced_text::Shaping::default(),
                        wrapping: advanced_text::Wrapping::None,
                    });
                    paragraph.min_width()
                });
                state.paragraph.update(advanced_text::Text {
                    content: &visible,
                    bounds,
                    size: self.size,
                    line_height: advanced_text::LineHeight::default(),
                    font,
                    align_x: advanced_text::Alignment::Default,
                    align_y: iced::alignment::Vertical::Top,
                    shaping: advanced_text::Shaping::default(),
                    wrapping: advanced_text::Wrapping::None,
                });
                self.content.clone_into(&mut state.source);
                state.width = bounds.width;
                state.size = self.size.0;
                state.font = self.font;
            }
            Size::new(bounds.width, state.paragraph.min_height())
        })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let Some(viewport) = layout.bounds().intersection(viewport) else {
            return;
        };
        advanced_text::Renderer::fill_paragraph(
            renderer,
            state.paragraph.raw(),
            layout.position(),
            self.color,
            viewport,
        );
    }
}

pub(crate) fn ellipsize_to_width(
    content: &str,
    max_width: f32,
    mut measure: impl FnMut(&str) -> f32,
) -> String {
    if measure(content) <= max_width {
        return content.to_owned();
    }
    if measure("…") > max_width {
        return String::new();
    }

    let boundaries = content
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(content.len()))
        .collect::<Vec<_>>();
    let mut low = 0;
    let mut high = boundaries.len() - 1;
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate = format!("{}…", &content[..boundaries[middle]]);
        if measure(&candidate) <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    format!("{}…", &content[..boundaries[low]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsis_uses_unicode_boundaries_and_the_actual_width() {
        let measured = |value: &str| value.chars().count() as f32;
        assert_eq!(ellipsize_to_width("ABCD", 3.0, measured), "AB…");
        assert_eq!(ellipsize_to_width("日本語", 2.0, measured), "日…");
        assert_eq!(ellipsize_to_width("AB", 2.0, measured), "AB");
        assert_eq!(ellipsize_to_width("AB", 0.0, measured), "");
    }

    #[test]
    fn long_session_titles_stay_inside_the_width_left_after_metadata() {
        let measured = |value: &str| {
            value
                .chars()
                .map(|character| if character.is_ascii() { 1.0 } else { 2.0 })
                .sum::<f32>()
        };
        for title in [
            "とても長い日本語のセッション名がmodel effort contextへ続く",
            "A session title that must stop before opus high 73 percent",
        ] {
            let visible = ellipsize_to_width(title, 18.0, measured);
            assert!(visible.ends_with('…'));
            assert!(measured(&visible) <= 18.0);
            assert!(title.is_char_boundary(visible.trim_end_matches('…').len()));
        }
    }
}
