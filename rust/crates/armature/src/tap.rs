//! 押されたことだけを知らせる薄い包み。中身をそのまま描き、左ボタンが包みの中で
//! 押されたら合図を出す。**中身が押下を捕まえても知らせる**——`mouse_area` は
//! 捕まえられた押下を見送るので、入力欄を押したことが分からない。
//! 合図を出したあとも押下は中身へそのまま渡す(入力欄はふつうに鍵を受け取る)。

use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Element, Event, Length, Point, Rectangle, Size, Theme, Vector};

pub struct Tap<'a, Message> {
    inner: Element<'a, Message>,
    on_press: Box<dyn Fn(Point, Size) -> Message + 'a>,
}

/// 中身を包む。左ボタンが押されたら `on_press(包み内の位置, 包みの寸法)` の合図を出す。
pub fn tap<'a, Message: 'a>(
    inner: impl Into<Element<'a, Message>>,
    on_press: impl Fn(Point, Size) -> Message + 'a,
) -> Element<'a, Message> {
    Element::new(Tap { inner: inner.into(), on_press: Box::new(on_press) })
}

impl<Message> Widget<Message, Theme, iced::Renderer> for Tap<'_, Message> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        self.inner.as_widget().tag()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        self.inner.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.inner.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.inner.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.inner.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.inner.as_widget().size_hint()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.inner.as_widget_mut().layout(tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.inner.as_widget().draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        self.inner.as_widget_mut().operate(tree, layout, renderer, operation);
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let bounds = layout.bounds();
            if let Some(position) = cursor.position_in(bounds) {
                shell.publish((self.on_press)(position, bounds.size()));
            }
        }
        self.inner
            .as_widget_mut()
            .update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.inner.as_widget().mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.inner.as_widget_mut().overlay(tree, layout, renderer, viewport, translation)
    }
}
