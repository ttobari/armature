//! Armature の下の層。窓(`armature` crate)を持たず、値を読む・書くだけ。
//!
//! | モジュール | 受け持ち |
//! |---|---|
//! | [`paths`] | アプリの身元と置き場(タスク・状態) |
//! | [`lang`] | 表示の言語(English / 日本語)と `tr!` |
//! | [`vault`] | タスクの md を読んで一覧の形に組む |
//! | [`board`] | 組んだ一覧の値(木・題・待ち・起動条件) |
//! | [`order`] | タスクの並び(`order.txt`) |
//! | [`write`] | タスクの md への書き(起票・状態・親・並び・起動条件) |
//! | [`monitor`] | Claude Code のセッションの読み取り(タブ・会話ログ・サブエージェント) |
//! | [`page_trans`] | アプリ内ブラウザの頁の翻訳 |
//! | [`holiday`] | 日本の祝日 |
//! | [`geo`] | 状態の置き場への薄い口 |
//! | [`proc`] | 子プロセスの起動口 |

pub mod board;
pub mod geo;
pub mod holiday;
pub mod lang;
pub mod monitor;
pub mod order;
pub mod page_trans;
pub mod paths;
pub mod proc;
pub mod vault;
pub mod write;
