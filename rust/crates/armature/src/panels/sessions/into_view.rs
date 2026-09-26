//! 一覧の中の1行を、**隠れているときだけ**窓へ入れる送り。
//!
//! ドラフトパネル(`drafts.rs`)は行が全部同じ高さなので「総高 ÷ 行数」で位置が出るが、
//! セッション一覧は群の見出し・配下の行・開いた小卓で高さがまちまちで、算術では
//! 当てられない。そこで実際のレイアウトから測る——`Operation` は木を丸ごと辿り、
//! **窓の外の行にも寄る**ので、隠れている行の位置もこの経路なら分かる。
//!
//! 測ってから送るまでを1つの操作で済ませる。`finish` で
//! [`Outcome::Chain`] を返すと、runtime がその場で次の操作(送り)を続けて走らせる。

use std::marker::PhantomData;

use iced::advanced::widget::operation::{Operation, Outcome, scrollable};
use iced::advanced::widget::{Id, operation};
use iced::{Rectangle, Vector};

/// `row` が `list` の窓から出ているときだけ、入れるのに要る最小限だけ送る操作。
///
/// 窓の中に見えている間は何もしない——選び直すたびに一覧が動くと、目で追って
/// いる並びがそのたびにずれる。
pub fn scroll_into_view<T: 'static>(list: Id, row: Id) -> impl Operation<T> {
    ScrollIntoView {
        list,
        row,
        window: None,
        row_bounds: None,
        message: PhantomData,
    }
}

struct ScrollIntoView<T> {
    list: Id,
    row: Id,
    /// 窓の矩形・中身の矩形・いまの送り。
    window: Option<(Rectangle, Rectangle, Vector)>,
    row_bounds: Option<Rectangle>,
    /// この操作は合図を出さない(送りは `Outcome::Chain` でその場で続ける)。
    /// 親の `Task<Message>` に混ぜられるよう、型だけ借りる。
    message: PhantomData<fn() -> T>,
}

impl<T> ScrollIntoView<T> {
    /// 送り先(px)。動かす要が無ければ `None`。
    fn target(&self) -> Option<f32> {
        let (bounds, content, translation) = self.window?;
        let row = self.row_bounds?;
        if content.height <= bounds.height {
            return None;
        }
        // 行の矩形も中身の矩形も送りを掛ける前の座標で来る。引けば「中身の
        // 上端から何px目か」になり、いまの送り(translation)と同じ物差しに乗る。
        let top = row.y - content.y;
        let bottom = top + row.height;
        let offset = translation.y;
        let target = if top < offset {
            top
        } else if bottom > offset + bounds.height {
            bottom - bounds.height
        } else {
            return None;
        };
        let target = target.clamp(0.0, (content.height - bounds.height).max(0.0));
        ((target - offset).abs() >= 0.5).then_some(target)
    }
}

impl<T: 'static> Operation<T> for ScrollIntoView<T> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn scrollable::Scrollable,
    ) {
        if id == Some(&self.list) {
            self.window = Some((bounds, content_bounds, translation));
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.row) {
            self.row_bounds = Some(bounds);
        }
    }

    fn finish(&self) -> Outcome<T> {
        let Some(target) = self.target() else {
            return Outcome::None;
        };
        Outcome::Chain(Box::new(operation::scrollable::scroll_to(
            self.list.clone(),
            operation::scrollable::AbsoluteOffset {
                x: None,
                y: Some(target),
            },
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 窓は高さ40・中身は高さ100・行は高さ10。上端から `y` 番目の行を測った体。
    fn measured(offset: f32, row_top: f32) -> ScrollIntoView<()> {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
        };
        let content = Rectangle {
            height: 100.0,
            ..bounds
        };
        ScrollIntoView {
            list: Id::unique(),
            row: Id::unique(),
            window: Some((bounds, content, Vector::new(0.0, offset))),
            row_bounds: Some(Rectangle {
                x: 0.0,
                y: row_top,
                width: 100.0,
                height: 10.0,
            }),
            message: PhantomData,
        }
    }

    #[test]
    fn a_row_inside_the_window_does_not_move_the_list() {
        assert_eq!(measured(0.0, 0.0).target(), None);
        assert_eq!(measured(0.0, 30.0).target(), None, "下辺ちょうどは中");
        assert_eq!(measured(20.0, 20.0).target(), None);
    }

    #[test]
    fn a_hidden_row_comes_back_by_the_smallest_step() {
        // 下へ1行はみ出す → 下辺に着くぶんだけ。
        assert_eq!(measured(0.0, 40.0).target(), Some(10.0));
        // 上へ出ている → その行の上端まで。
        assert_eq!(measured(30.0, 10.0).target(), Some(10.0));
        // 末尾は中身の終わりで止める(送りすぎない)。
        assert_eq!(measured(0.0, 90.0).target(), Some(60.0));
    }

    #[test]
    fn nothing_to_measure_means_nothing_to_scroll() {
        let mut unmeasured = measured(0.0, 40.0);
        unmeasured.window = None;
        assert_eq!(unmeasured.target(), None);

        let mut rowless = measured(0.0, 40.0);
        rowless.row_bounds = None;
        assert_eq!(rowless.target(), None);

        let mut fits = measured(0.0, 40.0);
        if let Some((bounds, content, translation)) = fits.window {
            fits.window = Some((
                bounds,
                Rectangle {
                    height: 20.0,
                    ..content
                },
                translation,
            ));
        }
        assert_eq!(fits.target(), None, "収まっているなら送らない");
    }
}
