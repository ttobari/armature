//! 光る字。**「いま」を指すものだけ**——時計・暦の今日・選んでいる行——に掛ける。
//!
//! 字の芯のまわりに、同じ字を薄く・少しずつずらして何周か重ねて滲みにする。
//! 後処理でぼかすのと違って画面を読み戻さないので、光る字が何個あっても描く手間は
//! 字の数に比例するだけで済む。滲みは押下を受けない(`update` は芯へだけ渡す)。

use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Renderer as _, Shell, Widget, layout, mouse, renderer};
use iced::{Color, Element, Event, Length, Rectangle, Size, Theme, Vector};

/// 滲みの輪。半径(論理px)・向きの数・1枚の濃さ。
/// 内側ほど濃く、外へ行くほど薄く広く——重なった所が芯のまわりで自然に落ちる。
const RINGS: [(f32, usize, f32); 3] = [(1.5, 8, 0.10), (3.5, 12, 0.05), (7.0, 16, 0.022)];

pub struct Glow<'a, Message> {
    core: Element<'a, Message>,
    rings: Vec<Element<'a, Message>>,
    strength: f32,
}

/// 字を光らせる。`make` は色を受けて同じ字を組む(芯と輪で色だけ変えて何度か呼ぶ)。
/// `strength` は滲みの濃さの倍率(1.0 が時計)。
pub fn glow<'a, Message: 'a>(
    make: impl Fn(Color) -> Element<'a, Message>,
    ink: Color,
    strength: f32,
) -> Element<'a, Message> {
    let rings = RINGS
        .iter()
        .map(|(_, _, alpha)| make(Color { a: (alpha * strength).min(1.0), ..ink }))
        .collect();
    Element::new(Glow {
        core: make(ink),
        rings,
        strength,
    })
}

impl<Message> Widget<Message, Theme, iced::Renderer> for Glow<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        std::iter::once(&self.core)
            .chain(&self.rings)
            .map(Tree::new)
            .collect()
    }

    fn diff(&self, tree: &mut Tree) {
        let children: Vec<&Element<'_, Message>> =
            std::iter::once(&self.core).chain(&self.rings).collect();
        tree.diff_children(&children);
    }

    fn size(&self) -> Size<Length> {
        self.core.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.core.as_widget().size_hint()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let core = self
            .core
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        let size = core.size();
        let mut nodes = vec![core];
        for (index, ring) in self.rings.iter_mut().enumerate() {
            nodes.push(
                ring.as_widget_mut()
                    .layout(&mut tree.children[index + 1], renderer, limits),
            );
        }
        layout::Node::with_children(size, nodes)
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
        let mut children = layout.children();
        let Some(core_layout) = children.next() else {
            return;
        };
        if self.strength > 0.0 {
            // 滲みは字の外へ半径ぶんはみ出す。見切れないよう描き先を広げて渡す。
            let halo = viewport.expand(RINGS[RINGS.len() - 1].0 + 1.0);
            for (index, ((ring, ring_layout), (radius, taps, _))) in
                self.rings.iter().zip(children).zip(RINGS).enumerate()
            {
                let ring_tree = &tree.children[index + 1];
                for tap in 0..taps {
                    #[allow(clippy::cast_precision_loss)]
                    let angle = std::f32::consts::TAU * tap as f32 / taps as f32;
                    let offset = Vector::new(radius * angle.cos(), radius * angle.sin());
                    renderer.with_translation(offset, |renderer| {
                        ring.as_widget().draw(
                            ring_tree,
                            renderer,
                            theme,
                            style,
                            ring_layout,
                            cursor,
                            &halo,
                        );
                    });
                }
            }
        }
        self.core
            .as_widget()
            .draw(&tree.children[0], renderer, theme, style, core_layout, cursor, viewport);
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        if let Some(core_layout) = layout.children().next() {
            self.core
                .as_widget_mut()
                .operate(&mut tree.children[0], core_layout, renderer, operation);
        }
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
        if let Some(core_layout) = layout.children().next() {
            self.core.as_widget_mut().update(
                &mut tree.children[0],
                event,
                core_layout,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        layout.children().next().map_or(mouse::Interaction::None, |core_layout| {
            self.core
                .as_widget()
                .mouse_interaction(&tree.children[0], core_layout, cursor, viewport, renderer)
        })
    }
}
