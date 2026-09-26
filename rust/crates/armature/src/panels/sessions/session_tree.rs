//! セッション一覧の枝(ツリー)を引く1本の線。パネルの行の下に、いま動いている
//! サブエージェントの行を吊るす。
//!
//! 以前は字のグリフ(`├─` / `└─`)で枝を組んでいた。字は行の中に
//! 収まるので、**行と行の隙間で縦線が切れる**——1行ぶんの高さより字のほうが低く、
//! 行間も空いているので、縦線が途切れ途切れの破線に見えた。太い罫線グリフの黒々しさも、目立たせたいはずの
//! 題より枝のほうを先に読ませていた。
//!
//! 直しは**線を字から絵へ移す**こと。行の高さいっぱいに 1px の縦線を引き、
//! 行の隙間を 0 にして、上下の行の線が継ぎ目なく繋がるようにする。横線(肘)は
//! 生死の印の手前で止め、最後の子だけ肘で縦線を止めて `└` を作る。
//!
//! ## 絵は矩形(`fill_quad`)で引く——canvas は使わない
//!
//! 最初の版(2026-09-06 午前)は行ごとに `canvas` を `stack!` で重ねていた。`Stack` は
//! 2 枚目以降の子を別の層に置き、iced_wgpu は**三角形(mesh)を持つ層ごとに描画パスを
//! 切る**——MSAA の面へ描いて resolve し、画面へ写す 1 パス、そのあと本線を `Load` で
//! 開き直す 1 パス。5K の窓ではパス 1 本ごとに面全体のタイルの読み書きが走るので、
//! 見えている行の数だけ 5K の面を 3 回なで直し、1 コマの GPU が 90ms に達していた。
//! 主線は毎コマ `nextDrawable` で 50〜77ms 眠り、打鍵がその後ろに並ぶ。
//!
//! 矩形(`renderer::Quad`)は同じ層でも本線のパスの中で描かれ、層が増えても scissor が
//! 変わるだけ。枝は縦線と肘の矩形 2 個で足りるので、こちらで引く。
//!
//! ## 桝取り(全部この 1 箇所で決める)
//!
//! ```text
//!   0 2 7.5 20.0
//!   │ │ │ │
//!   ├───┤ パネルの行の左詰め物(BUTTON_PAD_X)
//!       │ ● │ │ 親の印。縦線は**丸の墨の真下**
//!       │ ╵ │ │
//!       │ ├─────────○ 配下の印(桝 MARK_BOX = 11)
//!       │ STEM_X AGENT_ELBOW_END 肘は配下の丸の左端まで届いて止まる
//! ```
//!
//! 配下の印を親の印の桁から一定量だけ下げるだけで、モデル・エフォート・使用量の列は
//! 触らない——属性は行の右端から詰めるので、左を下げても桁はずれない。

use iced::advanced::widget;
use iced::advanced::{Layout, Widget, layout, renderer};
use iced::{Color, Element, Length, Point, Rectangle, Size, Vector, mouse};

/// 枝の太さ。1px より太くしない——枝は地図であって主役ではない。
pub const LINE: f32 = 1.0;

/// パネルの行(ボタン)の左右の詰め物。
pub const BUTTON_PAD_X: f32 = 2.0;
/// パネルの行(ボタン)の上下の詰め物。
///
/// **行の隙間はここへ寄せてある。**一覧の縦の間隔は「詰め物 + 隙間」で決まるが、
/// 隙間があると枝の縦線がそこで切れるので、隙間を 0 にして同じ量を詰め物へ移した。
/// 行の高さも一覧の総丈も以前と変わらない。
pub const BUTTON_PAD_Y: f32 = 6.0;

/// 生死の印(`⠋` / `●` / `○`)の桝。
pub const MARK_BOX: f32 = 11.0;

/// 頭の行の高さ。題(13px)の行送りで決まる。
const HEAD_LINE: f32 = 17.0;

/// 印の墨の中心が、桝の左からどれだけか。
///
/// **桝の中心と一致する**——ただしそれは `lib.rs::session_mark` が印を桝の中で
/// 中央に寄せているからで、放っておくと成り立たない。左詰めのままだと前送りの
/// 狭い明滅の字(`⠋`)だけが桝の左へ寄り、墨の中心が 3.75(丸は 5.0)に立って、
/// **縦線が丸から 1.75px ずれる**——の正体はこれ(検分台の画で列を数えた実測)。
/// 中央へ寄せたあとは `⠹` も `○` も `●` も墨の中心が桝の中心に乗る。
const MARK_INK_CENTRE: f32 = MARK_BOX / 2.0;

/// 縦線の中心の桁。**親の印の丸の真下**——枝が丸から生えて見える。
pub const STEM_X: f32 = BUTTON_PAD_X + MARK_INK_CENTRE;

/// 肘(横線)の高さ。頭の行の中心。
pub const ELBOW_Y: f32 = BUTTON_PAD_Y + HEAD_LINE / 2.0;

/// 親の印の下端。幹はここから下へ伸ばす(印の上に線を重ねない)。
pub const MARK_BOTTOM: f32 = ELBOW_Y + MARK_BOX / 2.0;

/// 配下(サブエージェント)の行頭に差す余白。枝そのものは行の上に重ねるので、
/// ここで取るのは幅だけ。
pub const AGENT_INDENT: f32 = 12.0;

/// 配下の行の丈。名(12px)の行送りが収まる最小の整数——**決め打つ**のは、
/// 縮む行だと枝の丈が字の高さに引きずられて上下の行と繋がらないため。
pub const AGENT_ROW: f32 = 16.0;

/// 配下(サブエージェント)の肘の右端。配下の印の丸の左端まで届かせる。
/// 幹([`STEM_X`])は親の印の真下。
pub const AGENT_ELBOW_END: f32 = 20.0;

/// 親から配下の行へ渡す継ぎ手の丈。配下の行は自前で縦線を持つので、幹は短い
/// 継ぎ手で足りる(下まで引くと、最後の配下で止まるはずの線が行の外へはみ出す)。
const TRUNK_STUB: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// 配下を持つパネル。印の下から配下の行へ渡す継ぎ手。
    Trunk,
    /// 配下(サブエージェント)の行。行の上に重ねて引く。
    Agent { last: bool },
}

/// 枝の絵。入力は捕まえない(`update` を実装していない)ので、行のボタンに
/// 重ねても押し心地は変わらない。
#[derive(Clone, Copy, Debug)]
pub struct Tree {
    shape: Shape,
    color: Color,
}

impl Tree {
    /// 配下を持つパネルの幹。
    #[must_use]
    pub fn trunk(color: Color) -> Self {
        Self {
            shape: Shape::Trunk,
            color,
        }
    }

    /// 配下(サブエージェント)の枝。`last` なら肘で縦線を止める。
    #[must_use]
    pub fn agent(color: Color, last: bool) -> Self {
        Self {
            shape: Shape::Agent { last },
            color,
        }
    }

    /// 引く矩形の列(`x, y, w, h`)。**寸法の答えはここ 1 箇所**——描く側も試験も
    /// 同じ列を見る。
    fn strokes(&self, bounds: Size) -> Vec<(f32, f32, f32, f32)> {
        match self.shape {
            Shape::Trunk => {
                let stem_x = STEM_X - LINE / 2.0;
                let joint = (MARK_BOTTOM + TRUNK_STUB).min(bounds.height) - MARK_BOTTOM;
                vec![(stem_x, MARK_BOTTOM, LINE, joint.max(0.0))]
            }
            Shape::Agent { last } => {
                // 配下の行に重ねる絵。行はパネルの詰め物の内側から始まるので、
                // 桁はパネルの座標からその詰め物ぶん左へ寄る。
                let stem_x = STEM_X - BUTTON_PAD_X - LINE / 2.0;
                let elbow_end = AGENT_ELBOW_END - BUTTON_PAD_X;
                // 肘は自分の行の真ん中。行の丈は `AGENT_ROW` で決め打ってあるが、
                // 実寸から取れば書体を変えても中心に乗る。
                let middle = (bounds.height / 2.0).round();
                let end = if last { middle + LINE / 2.0 } else { bounds.height };
                vec![
                    (stem_x, 0.0, LINE, end),
                    (stem_x, middle - LINE / 2.0, elbow_end - stem_x, LINE),
                ]
            }
        }
    }

    /// 引く矩形を `paint` へ 1 つずつ渡す。描く側([`Widget::draw`])も試験も同じ口を
    /// 通る——**ここから出るのは矩形だけ**で、三角形(mesh)は出ない(頭注)。
    fn paint(&self, size: Size, mut paint: impl FnMut(Rectangle, Color)) {
        for (x, y, w, h) in self.strokes(size) {
            paint(Rectangle::new(Point::new(x, y), Size::new(w, h)), self.color);
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Tree
where
    Renderer: renderer::Renderer,
{
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn draw(
        &self,
        _tree: &widget::Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        if bounds.intersection(viewport).is_none() {
            return;
        }
        let origin = Vector::new(bounds.x, bounds.y);
        self.paint(bounds.size(), |stroke, color| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: stroke + origin,
                    ..renderer::Quad::default()
                },
                color,
            );
        });
    }
}

impl<'a, Message, Theme, Renderer> From<Tree> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(tree: Tree) -> Self {
        Element::new(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: Size = Size {
        width: 300.0,
        height: 29.0,
    };

    fn ink() -> Color {
        Color::WHITE
    }

    /// 幹は印の丸の墨の桁に立つ——線が丸の真下から生えて見えるための条件。
    #[test]
    fn the_stem_stands_under_the_ink_of_the_mark() {
        assert_eq!(STEM_X, BUTTON_PAD_X + MARK_INK_CENTRE);
        let strokes = Tree::trunk(ink()).strokes(ROW);
        assert_eq!(strokes[0].0 + LINE / 2.0, STEM_X);
        // 配下の枝も同じ桁(行の詰め物ぶん左の座標で引く)。
        let agent = Tree::agent(ink(), false).strokes(Size::new(300.0, AGENT_ROW))[0].0;
        assert_eq!(strokes[0].0, agent + BUTTON_PAD_X);
    }

    /// 途中の配下は行を貫き、最後の配下は肘で止まる(`└`)。
    #[test]
    fn the_last_agent_stops_the_stem_at_the_elbow() {
        let row = Size::new(300.0, AGENT_ROW);
        let middle = Tree::agent(ink(), false).strokes(row);
        assert_eq!(middle[0].1, 0.0);
        assert_eq!(middle[0].3, row.height);
        let last = Tree::agent(ink(), true).strokes(row);
        assert!(last[0].1 + last[0].3 < row.height);
    }

    /// 肘は配下の印の丸まで届いて止まる——題の領分へは入らない。
    #[test]
    fn the_elbow_reaches_the_mark_and_stops() {
        let (x, _, w, _) = Tree::agent(ink(), false).strokes(Size::new(300.0, AGENT_ROW))[1];
        assert_eq!(x + w + BUTTON_PAD_X, AGENT_ELBOW_END);
        assert!(x < AGENT_ELBOW_END, "肘は左から右へ引く");
    }

    /// どの形の枝も、1 行で出すのは 1px 幅の矩形が高々 2 個——それ以外は何も出さない。
    /// 行ごとに三角形(mesh)の層を作って 5K の面を 3 回なで直していたのが、打鍵の遅れの
    /// 正体(頭注・2026-09-06)。矩形の数が行数に比例して増える以上には重くならない。
    #[test]
    fn a_row_paints_at_most_two_thin_rectangles_and_nothing_else() {
        for tree in [
            Tree::agent(ink(), false),
            Tree::agent(ink(), true),
            Tree::trunk(ink()),
        ] {
            let mut rectangles = 0;
            tree.paint(ROW, |stroke, color| {
                rectangles += 1;
                assert_eq!(color, ink());
                assert!(
                    stroke.width == LINE || stroke.height == LINE,
                    "{tree:?}: 太い矩形 {stroke:?}"
                );
            });
            assert!((1..=2).contains(&rectangles), "{tree:?}: 矩形 {rectangles} 個");
        }
    }
}
