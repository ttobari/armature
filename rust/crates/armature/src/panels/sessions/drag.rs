//! 掴んで動かす包み。中身をそのまま描き、左ボタンで押してから一定の距離を
//! 動いたら「掴んだ」、離したら「落とした」の合図を出す。
//!
//! **押しは包みの側で先に読む。**`mouse_area` の `on_press` は中に居る `button` に
//! 食われる——button は押しを `capture_event` してから返すので、外側の
//! `mouse_area` まで届かない(iced 0.14 の実装)。
//!
//! **落とす先はここでは報せない。**行ごとの `mouse_area` の `on_enter` が
//! 「いまどの行に乗っているか」を既に持っているので、包みは掴んだ・落としたの
//! 2点だけを出す。位置を `on_move` で報せると指が乗っている間ずっと窓が
//! 描き直される([`crate::tap`] と同じ理由)。
//!
//! **離しは捕まえない。**捕まえると中の button の `is_pressed` が立ったまま
//! 残り、以後ホバーするだけで押された面になる。掴まずに離した(=ただの click)
//! ときは button の選択がそのまま通ってよいので、素通しにする。

use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Element, Event, Length, Point, Rectangle, Size, Theme, Vector};

/// 掴んだと見なす距離(論理px)。
///
/// **行の高さ(約24px)の半分弱を取る。**4px では選ぶつもりの押しが掴みに化けて、
/// 手が滑ったぶんだけ並びが動いた。掴むのは
/// 「動かすつもりで動かした」ときだけでよく、ここは鈍いほうが正しい。
const THRESHOLD: f32 = 10.0;

#[derive(Default)]
struct State {
    /// 左ボタンを押した位置。離すまで持つ。
    origin: Option<Point>,
    dragging: bool,
}

pub struct Drag<'a, Message> {
    inner: Element<'a, Message>,
    on_grab: Message,
    on_drop: Message,
}

/// 中身を包む。押してから [`THRESHOLD`] 動いたら `on_grab`、離したら `on_drop`。
pub fn drag<'a, Message: Clone + 'a>(
    inner: impl Into<Element<'a, Message>>,
    on_grab: Message,
    on_drop: Message,
) -> Element<'a, Message> {
    Element::new(Drag {
        inner: inner.into(),
        on_grab,
        on_drop,
    })
}

impl<Message: Clone> Widget<Message, Theme, iced::Renderer> for Drag<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.inner)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.inner));
    }

    fn size(&self) -> Size<Length> {
        self.inner.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.inner.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.inner
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
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
        self.inner.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.inner
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
        {
            let state: &mut State = tree.state.downcast_mut();
            match event {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    if cursor.is_over(layout.bounds()) {
                        state.origin = cursor.position();
                        state.dragging = false;
                    }
                }
                Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                    if let Some(origin) = state.origin
                        && let Some(position) = cursor.position()
                        && !state.dragging
                        && origin.distance(position) > THRESHOLD
                    {
                        state.dragging = true;
                        shell.publish(self.on_grab.clone());
                    }
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    let dragging = std::mem::take(&mut state.dragging);
                    state.origin = None;
                    if dragging {
                        shell.publish(self.on_drop.clone());
                    }
                }
                _ => {}
            }
        }
        self.inner.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state: &State = tree.state.downcast_ref();
        if state.dragging {
            return mouse::Interaction::Grabbing;
        }
        self.inner
            .as_widget()
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.inner.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
