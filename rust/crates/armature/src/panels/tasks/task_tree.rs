//! タスクパネルの枝(バンドルとその配下)を引く1本の線。
//!
//! セッション一覧([`crate::session_tree`])・ドラフト一覧([`crate::drafts_tree`])と
//! 同じ作法——**線は字ではなく絵**。字のグリフ(`├─`)は行の中にしか置けないので
//! 行間で縦線が切れるが、行の上に重ねた絵なら行の高さいっぱいに引けて、上下の行と
//! 継ぎ目なく繋がる。色も同じで、生死や意味の色を借りず `text_faint` を薄めた1色だけ
//! 使う——**枝は地図であって主役ではない**。読ませたいのは題。
//!
//! 絵は矩形(`fill_quad`)で引き、**canvas は使わない**(2026-09-06 に canvas の層が
//! 5K の面を何度もなで直して打鍵が遅れた。詳しくは `session_tree` の頭注)。
//!
//! ## 先の2つと違う点——深さが限りない
//!
//! セッションもドラフトも木は2段で止まるので、形を `Trunk` / `Branch` と数え上げて
//! 桁を定数で決め打てた。タスクの木はバンドルが何段でも入れ子になる(`board.rs` の
//! `build_node` は深さを数えない)ので、桁は `depth` から算術で出し、通す縦線は
//! **行ごとに「どの列がまだ下へ続くか」の真偽の列**([`Tree::new`] の `stems`)で渡す。
//!
//! ## 桝取り(全部この 1 箇所で決める)
//!
//! ```text
//!   5      10.5              24.5
//!   │       │                 │
//!   ├───────┤ 行の左詰め物(PAD_X)
//!           │  │ バンドルの見出し(桝 MARK_BOX)。幹は印の墨の真下から
//!           │ │ │
//!           │ └─┼───┤ ・ │ 1131 …… 配下の行。肘は子の印の桝の左端で止まる
//!         STEM(0)      STEM(1)
//!           ├─────────┤ 深さ1段(INDENT)
//! ```
//!
//! 肘が届く先は**子の印の桝の左端**。印の中身(`・` / `●` / `○`)が線を受け止めるので、
//! 素の行でも `・` を必ず出す——空の桝で肘を止めると、線が宙で切れて見える。

use iced::advanced::widget;
use iced::advanced::{Layout, Widget, layout, renderer};
use iced::{Color, Element, Length, Point, Rectangle, Size, Vector, mouse};

/// 枝の太さ。1px より太くしない。
pub const LINE: f32 = 1.0;

/// 行(ボタン)の左右の詰め物。**見出しも素の行も同じ**——違う値にすると印の桁が
/// ずれ、幹が子の印の真上を通らない。
pub const PAD_X: f32 = 5.0;
/// 行(ボタン)の上下の詰め物。
///
/// **行の隙間は 0**(`column![].spacing(0)`)——隙間があるとそこで縦線が切れる。
/// 以前の見た目にあった行間は、この詰め物へ寄せてある。
pub const PAD_Y: f32 = 3.0;

/// 行頭の印(束の字 / `·` / `●` / `○` / `◆`)の桝。
pub const MARK_BOX: f32 = 11.0;

/// バンドルの印(`tasks::BUNDLE_MARK` の束の字)の**墨の丈**の半分。桝(11)では
/// なく字そのものの高さで測る——桝の半分から幹を始めると印と線の間が空いて、
/// そこで線が切れて見える(`drafts_tree::MARK_INK` と同じ理由)。
///
/// **印の級数を変えたらここも動かす。** 2026-09-14 に印を 8px へ落としたとき、
/// 畳み印 `▾` 時代の 4.0 のままだと墨の下に 2px(実寸)の切れができた
/// ——検分台の画で列を数えて 3.0 に合わせてある。
pub const MARK_INK: f32 = 3.0;

/// 深さ1段ぶんの字下げ。
pub const INDENT: f32 = 14.0;

/// 深さ `depth` の行の、印の桝の左端。
#[must_use]
pub fn mark_x(depth: usize) -> f32 {
    PAD_X + INDENT * depth as f32
}

/// 深さ `depth` の節が、その配下へ下ろす縦線の桁。**自分の印の真下**。
#[must_use]
pub fn stem_x(depth: usize) -> f32 {
    mark_x(depth) + MARK_BOX / 2.0
}

/// 枝の絵。入力は捕まえない(`update` を実装していない)ので、行のボタンに重ねても
/// 押し心地は変わらない。
#[derive(Clone, Debug)]
pub struct Tree {
    /// 列ごとに「この行を貫いて下へ続くか」。長さ = その行の深さ。
    /// 末尾(自分の枝の列)だけは意味が違い、`false` なら末子——肘で止める。
    stems: Vec<bool>,
    /// 自分の印の下から行の下端へ渡す幹。配下を持つ見出しだけが立てる。
    trunk: bool,
    color: Color,
}

impl Tree {
    /// `stems` は列 0..depth の「下へ続くか」。`trunk` は配下を持つかどうか。
    /// 枝を1本も引かない行(深さ 0 で配下なし)では [`Tree::is_empty`] が真になる。
    #[must_use]
    pub fn new(stems: Vec<bool>, trunk: bool, color: Color) -> Self {
        Self {
            stems,
            trunk,
            color,
        }
    }

    /// 引く矩形が1つも無い行。重ねる意味が無いので、呼ぶ側は素の行だけを置く。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stems.is_empty() && !self.trunk
    }

    /// 引く矩形の列(`x, y, w, h`)。**寸法の答えはここ 1 箇所**——描く側も試験も
    /// 同じ列を見る。
    fn strokes(&self, bounds: Size) -> Vec<(f32, f32, f32, f32)> {
        // 肘は行の真ん中。丈を実寸から取るので、字の大きさを変えても中心に乗る。
        let middle = (bounds.height / 2.0).round();
        let depth = self.stems.len();
        let mut strokes = Vec::with_capacity(depth + 2);
        for (column, continues) in self.stems.iter().copied().enumerate() {
            let x = stem_x(column) - LINE / 2.0;
            if column + 1 == depth {
                // 自分の枝。末子なら肘で止め(`└`)、そうでなければ行を貫く。
                let end = if continues {
                    bounds.height
                } else {
                    middle + LINE / 2.0
                };
                strokes.push((x, 0.0, LINE, end));
                // 肘は自分の印の桝の左端まで届いて止まる——題の領分へは入らない。
                strokes.push((x, middle - LINE / 2.0, mark_x(depth) - x, LINE));
            } else if continues {
                strokes.push((x, 0.0, LINE, bounds.height));
            }
        }
        if self.trunk {
            // 印の墨のすぐ下から行の下端まで。次の行の枝が 0 から引き継ぐ。
            let x = stem_x(depth) - LINE / 2.0;
            let top = middle + MARK_INK;
            strokes.push((x, top, LINE, (bounds.height - top).max(0.0)));
        }
        strokes
    }

    /// 引く矩形を `paint` へ1つずつ渡す。描く側([`Widget::draw`])も試験も同じ口を
    /// 通る——**ここから出るのは矩形だけ**で、三角形(mesh)は出ない(頭注)。
    fn paint(&self, size: Size, mut paint: impl FnMut(Rectangle, Color)) {
        for (x, y, w, h) in self.strokes(size) {
            paint(
                Rectangle::new(Point::new(x, y), Size::new(w, h)),
                self.color,
            );
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
        height: 23.0,
    };

    fn ink() -> Color {
        Color::WHITE
    }

    /// 深さが増えるぶんだけ桁が右へ寄る。**算術で出す**のがこのパネルの要
    /// ——木は何段でも入れ子になる。
    #[test]
    fn the_columns_step_right_by_one_indent_per_depth() {
        assert_eq!(mark_x(0), PAD_X);
        assert_eq!(mark_x(2) - mark_x(1), INDENT);
        assert_eq!(stem_x(0), PAD_X + MARK_BOX / 2.0);
        assert_eq!(stem_x(3) - stem_x(2), INDENT);
    }

    /// 途中の子は行を貫く。**上下の行の線が継ぎ目なく繋がる**のはこの 1 点で決まる。
    #[test]
    fn a_middle_child_draws_its_stem_through_the_whole_row() {
        let strokes = Tree::new(vec![true], false, ink()).strokes(ROW);
        assert_eq!(strokes[0].1, 0.0);
        assert_eq!(strokes[0].3, ROW.height);
    }

    /// 末子は肘で止まる(`└`)。行の下へ線を垂らさない。
    #[test]
    fn the_last_child_stops_its_stem_at_the_elbow() {
        let strokes = Tree::new(vec![false], false, ink()).strokes(ROW);
        assert!(strokes[0].1 + strokes[0].3 < ROW.height);
    }

    /// 肘は自分の印の桝の左端で止まる——題の領分へは入らない。
    #[test]
    fn the_elbow_reaches_the_mark_box_and_stops() {
        let strokes = Tree::new(vec![true, false], false, ink()).strokes(ROW);
        let elbow = strokes.iter().find(|stroke| stroke.3 == LINE).expect("肘");
        assert_eq!(elbow.0 + elbow.2, mark_x(2));
        assert!(elbow.0 < mark_x(2), "肘は左から右へ引く");
    }

    /// 祖先の列は、まだ下に兄弟が残っているときだけ通す。閉じた列は引かない
    /// ——引くと、終わったはずの束の線が下の束まで伸びる。
    #[test]
    fn only_the_ancestors_that_continue_are_drawn_through() {
        let open = Tree::new(vec![true, true, false], false, ink()).strokes(ROW);
        let shut = Tree::new(vec![false, false, false], false, ink()).strokes(ROW);
        assert_eq!(open.len(), 4, "祖先2本 + 自分の枝 + 肘");
        assert_eq!(shut.len(), 2, "自分の枝 + 肘だけ");
    }

    /// 配下を持つ見出しは、印の墨の下から行の下端まで幹を下ろす。次の行の枝が
    /// `y = 0` から引き継ぐので、ここが欠けると見出しと最初の子の間で線が切れる。
    #[test]
    fn a_header_with_children_lowers_a_trunk_to_the_bottom_of_its_row() {
        let strokes = Tree::new(Vec::new(), true, ink()).strokes(ROW);
        assert_eq!(strokes.len(), 1);
        let (x, y, _, height) = strokes[0];
        assert_eq!(x + LINE / 2.0, stem_x(0));
        assert_eq!(y + height, ROW.height);
        assert!(y > ROW.height / 2.0, "印の墨より下から生やす");
    }

    /// 枝を1本も持たない行(深さ 0 の素のタスク)は空。重ねる意味が無い。
    #[test]
    fn a_lone_root_row_paints_nothing() {
        let tree = Tree::new(Vec::new(), false, ink());
        assert!(tree.is_empty());
        assert!(tree.strokes(ROW).is_empty());
    }

    /// どの行も出すのは 1px 幅(または丈)の矩形だけ——三角形(mesh)は出さない。
    /// 行ごとに mesh の層を作ると 5K の面をなで直して打鍵が遅れる(頭注・2026-09-06)。
    #[test]
    fn a_row_paints_only_thin_rectangles() {
        let shapes = [
            Tree::new(vec![true], false, ink()),
            Tree::new(vec![false], true, ink()),
            Tree::new(vec![true, false, true], true, ink()),
        ];
        for tree in shapes {
            let mut rectangles = 0;
            tree.paint(ROW, |stroke, color| {
                rectangles += 1;
                assert_eq!(color, ink());
                assert!(
                    stroke.width == LINE || stroke.height == LINE,
                    "{tree:?}: 太い矩形 {stroke:?}"
                );
            });
            assert!(rectangles > 0, "{tree:?}: 何も出ていない");
        }
    }
}
