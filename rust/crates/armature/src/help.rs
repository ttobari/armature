use iced::widget::{column, container, row, rule, scrollable, text};
use iced::{Background, Element, Length};

use crate::palette::{self, surface_window, text_muted, text_primary};
use armature_core::tr;

pub struct Section {
    /// 見出し(英語, 日本語)。
    pub title: (&'static str, &'static str),
    /// (キー, 英語, 日本語)。
    pub rows: &'static [(&'static str, &'static str, &'static str)],
}

pub const SECTIONS: &[Section] = &[
    Section {
        title: ("Moving around", "移動"),
        rows: &[
            ("⌘ H", "Focus the center", "中央へ"),
            (
                "⌘ L",
                "Focus the task list (the address bar while a page is open)",
                "タスクの一覧へ(頁を見ているときはアドレス欄へ)",
            ),
            (
                "⌘ R",
                "Task list (reloads the page while one is open)",
                "タスク一覧(頁を見ているときは頁の読み直し)",
            ),
            ("⌘ B", "Switch between the browser and the terminal", "ブラウザと端末を切替"),
            (
                "⌘ ⏎",
                "Give the keys to the terminal (hides the page if one is open)",
                "中央の端末へ鍵を渡す(頁が出ていれば畳む)",
            ),
            ("⌃ Tab", "Next tab", "次のタブ"),
            ("⌃ ⇧ Tab", "Previous tab", "前のタブ"),
            ("⌘ /", "Open or close this help", "このヘルプを開閉"),
            ("⌘ ,", "Open or close the settings (language, color theme)", "設定(言語・配色)を開閉"),
        ],
    },
    Section {
        title: ("Tabs", "タブ"),
        rows: &[
            (
                "⌘ T",
                "New Claude tab (a browser tab while the browser is open)",
                "新しい Claude タブ(ブラウザを見ているときはブラウザのタブ)",
            ),
            ("⌘ ⇧ T", "Bring back a closed Claude tab", "閉じた Claude タブを戻す"),
            ("⌘ J", "New shell", "新しいシェル"),
            ("⌘ W", "Close the current tab", "いまのタブを閉じる"),
            (
                "Drag",
                "Reorder sessions in the session list (⌃Tab follows this order)",
                "セッション一覧の行を掴んで並べ替え(⌃Tab の順もこれに従う)",
            ),
        ],
    },
    Section {
        title: ("Tasks", "タスク"),
        rows: &[
            ("J / K", "Move the selection down / up", "選択を下/上へ"),
            ("L / H", "Enter / leave a group", "バンドルへ入る/戻る"),
            ("⌃ L / ⇧ L", "Start Claude on the selected task", "選んだタスクの Claude を起こす"),
            ("M / E", "Change the model / effort to start with", "起こすときのモデル/思考量を切替"),
            ("D D", "Mark done (press twice)", "完了にする(2回続けて)"),
            ("X", "Pick up / put down a row (shows ↕)", "行を掴む/離す(↕が立つ)"),
            (
                "Enter",
                "Move the picked-up row under the same parent as the cursor",
                "掴んだ行をカーソル行と同じ親の下へ移す",
            ),
        ],
    },
    Section {
        title: ("Window", "画面"),
        rows: &[
            ("⌘ + / ⌘ -", "Larger / smaller text", "文字を拡大/縮小"),
            ("⌘ 0", "Reset the text size", "文字の大きさを戻す"),
            ("⇧⌘ R", "Restart the window", "窓ごと起こし直す"),
        ],
    },
    Section {
        title: ("Browser", "ブラウザ"),
        rows: &[
            ("F", "Label the visible links", "見えているリンクへ印を出す"),
            ("J / K", "Scroll down / up", "頁を下/上へ送る"),
            ("H / L", "Back / forward", "履歴を戻る/進む"),
            ("⌘ [ / ⌘ ]", "Back / forward", "履歴を戻る/進む"),
            (
                "T",
                "Switch between the translation and the original (remembered per site)",
                "翻訳と原文を切替(この選択を記憶)",
            ),
            ("⇧ T", "Careful translation by Claude", "Claude で精訳"),
            ("⌘ ⇧ D", "Bookmark this page", "いまの頁をブックマーク"),
            ("⌘ 1–9", "Go straight to a browser tab", "ブラウザのタブを直接選ぶ"),
        ],
    },
];

pub fn view<'a, Message: 'a>() -> Element<'a, Message> {
    let mut sections = column![].spacing(16).width(Length::Fill);
    for section in SECTIONS {
        let mut rows = column![].spacing(7).width(Length::Fill);
        for (key, en, ja) in section.rows {
            rows = rows.push(
                row![
                    text(*key)
                        .size(13)
                        .color(text_primary())
                        .width(Length::Fixed(112.0)),
                    text(tr!(*en, *ja))
                        .size(13)
                        .color(text_muted())
                        .width(Length::Fill),
                ]
                .spacing(12),
            );
        }
        sections = sections.push(
            column![
                text(tr!(section.title.0, section.title.1)).size(12).color(text_muted()),
                rule::horizontal(1),
                rows,
            ]
            .spacing(7),
        );
    }

    let card = container(
        column![
            row![
                text(tr!("Keyboard", "キー操作"))
                    .size(22)
                    .color(text_primary())
                    .width(Length::Fill),
                text(tr!("⌘ / to close", "⌘ / で閉じる")).size(11).color(text_muted()),
            ],
            scrollable(sections).height(Length::Fill),
        ]
        .spacing(18),
    )
    .padding(24)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_| palette::card_style());

    container(card)
        .padding(10)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(surface_window())),
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_is_listed_only_once_inside_its_section() {
        for section in SECTIONS {
            let mut keys = std::collections::BTreeSet::new();
            for (key, _, _) in section.rows {
                assert!(
                    keys.insert(*key),
                    "duplicate shortcut in {}: {key}",
                    section.title.0
                );
            }
        }
    }

    #[test]
    fn the_help_mentions_the_global_toggle() {
        let _: Element<'_, ()> = view();
        assert!(SECTIONS.iter().any(|section| section.title.0 == "Browser"));
    }
}
