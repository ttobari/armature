//! タスク一覧の値。[`crate::vault::Vault::data`] が組んだ JSON を読む。**読むだけ**
//! ——書き口は [`crate::write`] の領分。
//!
//! ## 一覧の形
//!
//! ```text
//!   ルート(root) … roots のうち deadRoots でないもの(並びは order.txt)
//!     ├ メンバー行 … そのバンドル直属のタスク(members)
//!     └ 子バンドル箱 … 入れ子(children・再帰)
//! ```
//!
//! ## 箱にするかどうか
//!
//! 分かれ目は「ルートで、かつ束でも直属タスク持ちでもないか」。`bundles`(子を
//! 1件でも持つIDの正本)に載っていれば、生きた子が全員居なくなっても箱のまま残す
//! ——`children`/`members` は生きた子しか載らないので、この2つだけを見ると
//! 「子が全員完了した束」が素の1行へ落ちる。
//!
//! 中身が空のバンドルは枠だけの箱として描き、選択できる空行を1本だけ入れる。

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// タスク一覧の取り直し間隔。
pub const POLL_INTERVAL: Duration = Duration::from_secs(4);
/// 空行のIDに付ける接尾辞。
///
/// 実在タスクのIDは常に数字文字列なので、この接尾辞付き文字列と衝突しない。
/// この接尾辞を持つIDは「選択カーソルを乗せられる」だけの存在で、実操作
/// (起動・完了化・移動)の対象にしてはならない。
pub const EMPTY_ROW_SUFFIX: &str = "::empty";

/// [`EMPTY_ROW_SUFFIX`] を持つ実体の無い行か。
#[must_use]
pub fn is_placeholder(id: &str) -> bool {
    id.ends_with(EMPTY_ROW_SUFFIX)
}

/// タスク1件。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Task {
    pub title: String,
    /// 状態("進行" / "完了")。
    pub state: String,
    /// まだ解けていない `待ち`(`vault::open_waiting` が解決済みの値)。
    /// 空なら着手可——**ここが「今すぐやれるか」の正本**。
    pub waiting: String,
}

/// バンドル直属のメンバー1件。`members` は `{"id": ..., "title": ...}` の配列で来る。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    pub title: String,
}

/// 一覧の中身ひと揃い。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    /// ルートIDの並び。
    pub roots: Vec<String>,
    /// 全員完了で畳まれたルート(描画しない)。
    pub dead_roots: BTreeSet<String>,
    /// 束かどうかの正本(子を1件でも持つID・完了の子も数える)。
    pub bundles: BTreeSet<String>,
    /// ID → 子バンドルのID。
    pub children: BTreeMap<String, Vec<String>>,
    /// ID → 子の並び(素タスクと子バンドルを混ぜた並び順)。
    /// `members` と `children` に分けると相対順が失われるため、順の正本はこちら。
    pub child_order: BTreeMap<String, Vec<String>>,
    /// ID → 直属タスク。
    pub members: BTreeMap<String, Vec<Member>>,
    /// ID → 見出し(全項目の題)。
    pub tag_label: BTreeMap<String, String>,
    /// ID → タスク。
    pub tasks: BTreeMap<String, Task>,
    /// 起動条件(`{"model": …, "effort": …}`)を丸ごと。手元で1欄だけ差し替えて
    /// 書き戻せるように全部持っておく。
    pub ui: serde_json::Value,
}

impl Board {
    /// [`crate::vault::Vault::data`] の JSON を読む。
    #[must_use]
    pub fn parse(json: &str) -> Option<Self> {
        let data: serde_json::Value = serde_json::from_str(json).ok()?;
        if !data.is_object() {
            return None;
        }
        Some(Self {
            roots: strings(&data["roots"]),
            dead_roots: strings(&data["deadRoots"]).into_iter().collect(),
            bundles: strings(&data["bundles"]).into_iter().collect(),
            children: data["children"]
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), strings(v))).collect())
                .unwrap_or_default(),
            child_order: data["childOrder"]
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), strings(v))).collect())
                .unwrap_or_default(),
            members: data["members"]
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), members_of(v))).collect())
                .unwrap_or_default(),
            tag_label: str_map(&data["tagLabel"]),
            tasks: data["tasks"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(id, t)| {
                            (
                                id.clone(),
                                Task {
                                    title: t["title"].as_str().unwrap_or_default().to_string(),
                                    state: t["state"].as_str().unwrap_or_default().to_string(),
                                    waiting: t["waiting"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .to_string(),
                                },
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ui: if data["ui"].is_object() {
                data["ui"].clone()
            } else {
                serde_json::json!({})
            },
        })
    }

    /// `ui` の1欄。
    #[must_use]
    pub fn ui_field(&self, key: &str) -> &str {
        self.ui[key].as_str().unwrap_or_default()
    }

    /// 起動に使うモデル。
    #[must_use]
    pub fn model(&self) -> &str {
        self.ui_field("model")
    }

    /// 起動に使う effort。
    #[must_use]
    pub fn effort(&self) -> &str {
        self.ui_field("effort")
    }

    /// `ui` の1欄を差し替えた新しい `ui`(そのまま [`crate::write::save_launch_settings`] へ渡す)。
    #[must_use]
    pub fn ui_with(&self, key: &str, value: &str) -> serde_json::Value {
        let mut ui = self.ui.clone();
        if !ui.is_object() {
            ui = serde_json::json!({});
        }
        ui[key] = serde_json::json!(value);
        ui
    }

    /// 描くルートの並び(畳まれたルートを除く)。
    #[must_use]
    pub fn live_roots(&self) -> Vec<String> {
        self.roots
            .iter()
            .filter(|r| !self.dead_roots.contains(*r))
            .cloned()
            .collect()
    }

    /// 表示に使う名前。
    ///
    /// `tasks` の題 → 呼び出し元が `members` から持っている題 → `tagLabel` →
    /// ID自身、の順に落ちる。
    #[must_use]
    pub fn label_of(&self, id: &str, fallback: Option<&str>) -> String {
        if let Some(task) = self.tasks.get(id)
            && !task.title.is_empty()
        {
            return task.title.clone();
        }
        if let Some(fallback) = fallback.filter(|f| !f.is_empty()) {
            return fallback.to_string();
        }
        self.tag_label
            .get(id)
            .filter(|l| !l.is_empty())
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    /// 着手できない(解けていない**外部要因の** `待ち` が残っている)か。
    ///
    /// 「外部要因」は利用者にもどうにもできないもの——他人の返事・レビュー、環境・機体、
    /// 外部システム。`自分` で始まる札(`自分` / `自分 (Aさんへの相談)` 等)は
    /// **利用者が動けば解ける**ので待ちに数えない。そちらは [`Self::is_your_turn`]。
    #[must_use]
    pub fn is_blocked(&self, id: &str) -> bool {
        waiting_tokens(self.waiting_on(id)).any(|token| !is_own_token(token))
    }

    /// 利用者の番か——解けていない札が **`自分` の札だけ**で、それ以外に止めているものが無い。
    /// 外部要因の札が1つでも混ざれば `false`(そのときは [`Self::is_blocked`] が立つ)。
    #[must_use]
    pub fn is_your_turn(&self, id: &str) -> bool {
        let mut tokens = waiting_tokens(self.waiting_on(id));
        let Some(first) = tokens.next() else {
            return false;
        };
        is_own_token(first) && tokens.all(is_own_token)
    }

    /// 待っている相手。空なら着手可。
    #[must_use]
    pub fn waiting_on(&self, id: &str) -> &str {
        self.tasks.get(id).map_or("", |task| task.waiting.as_str())
    }

    /// 子バンドルも直属タスクも持たず、束の正本にも載っていないか
    /// (ルートを箱にせず素の1行で出す判定)。
    #[must_use]
    pub fn is_truly_childless(&self, id: &str) -> bool {
        !self.bundles.contains(id)
            && self.children.get(id).is_none_or(Vec::is_empty)
            && self.members.get(id).is_none_or(Vec::is_empty)
    }
}

fn strings(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn members_of(value: &serde_json::Value) -> Vec<Member> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|item| match item {
                    serde_json::Value::String(s) => Some(Member {
                        id: s.clone(),
                        title: String::new(),
                    }),
                    other => Some(Member {
                        id: other["id"].as_str()?.to_string(),
                        title: other["title"].as_str().unwrap_or_default().to_string(),
                    }),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn str_map(value: &serde_json::Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// 一覧の1つの節(描画と選択の単位)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// 箱(バンドル)。中身は `body`。
    Box { id: String, body: Vec<Node> },
    /// タスクの行。`title` は `members` が持っていた題(無ければ空)。
    Row { id: String, title: String },
    /// 空のバンドルに入れる選択可能な空行。
    Empty { id: String },
}

impl Node {
    /// この節のID。
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Box { id, .. } | Self::Row { id, .. } | Self::Empty { id } => id,
        }
    }
}

/// ルートから木を組む。循環は打ち切る(壊れた入力で無限に潜らない)。
#[must_use]
pub fn build_node(board: &Board, id: &str, title: &str, depth: usize, path: &mut Vec<String>) -> Node {
    if path.iter().any(|p| p == id) {
        // 循環参照。素の行に潰す。
        return Node::Row {
            id: id.to_string(),
            title: title.to_string(),
        };
    }
    // 束でも直属タスク持ちでもないルートは箱にせず素の1行。
    if depth == 0 && board.is_truly_childless(id) {
        return Node::Row {
            id: id.to_string(),
            title: title.to_string(),
        };
    }
    path.push(id.to_string());
    let members: &[Member] = board.members.get(id).map_or(&[], Vec::as_slice);
    let kids: &[String] = board.children.get(id).map_or(&[], Vec::as_slice);
    let mut body: Vec<Node> = Vec::with_capacity(members.len() + kids.len());
    let mut placed: BTreeSet<&str> = BTreeSet::new();
    // 素タスクを先に全部・子バンドルを後に全部、で並べると並びのファイルで指定した
    // 作業順が壊れる(束が必ず末尾へ回る)。順の正本は child_order。
    for cid in board.child_order.get(id).map_or(&[][..], Vec::as_slice) {
        if let Some(m) = members.iter().find(|m| &m.id == cid) {
            body.push(Node::Row {
                id: m.id.clone(),
                title: m.title.clone(),
            });
            placed.insert(cid.as_str());
        } else if kids.iter().any(|k| k == cid) {
            body.push(build_node(board, cid, "", depth + 1, path));
            placed.insert(cid.as_str());
        }
    }
    // child_order に載らない子(完了バンドルの中抜きで繰り上がった子など)を落とさない。
    for m in members {
        if !placed.contains(m.id.as_str()) {
            body.push(Node::Row {
                id: m.id.clone(),
                title: m.title.clone(),
            });
        }
    }
    for kid in kids {
        if !placed.contains(kid.as_str()) {
            body.push(build_node(board, kid, "", depth + 1, path));
        }
    }
    if body.is_empty() {
        // 中身が本当に空のときだけ、選択可能な空行を1本。
        body.push(Node::Empty {
            id: format!("{id}{EMPTY_ROW_SUFFIX}"),
        });
    }
    path.pop();
    Node::Box {
        id: id.to_string(),
        body,
    }
}

/// 一覧ぶんの木(ルートを縦1本に並べたもの)。
#[must_use]
pub fn build_roots(board: &Board) -> Vec<Node> {
    board
        .live_roots()
        .iter()
        .map(|id| build_node(board, id, "", 0, &mut Vec::new()))
        .collect()
}

/// 選択できる節を画面表示順に並べる(箱の見出しは中身より前)。
pub fn flatten(nodes: &[Node], out: &mut Vec<String>) {
    for node in nodes {
        out.push(node.id().to_string());
        if let Node::Box { body, .. } = node {
            flatten(body, out);
        }
    }
}

/// `待ち` の札を1枚ずつ。区切りは `vault::open_waiting` と同じ(半角・全角のカンマ)。
fn waiting_tokens(raw: &str) -> impl Iterator<Item = &str> {
    raw.split([',', '、'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

/// 自分の番の札か。`自分` 単独も、`自分 (設計の承認)` のように理由を添えた形も同じ。
/// 英語の `me` も同じに扱う。
fn is_own_token(token: &str) -> bool {
    token.starts_with("自分") || token.eq_ignore_ascii_case("me") || token.to_ascii_lowercase().starts_with("me ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 沈める(⊘)のは利用者にもどうにもできない外部要因だけ。`自分` の札は「利用者の番」で、
    /// 外部要因が1枚でも混ざれば待ちの側へ倒れる。
    #[test]
    fn only_external_waits_block_while_own_waits_are_your_turn() {
        let mut board = Board::default();
        for (id, waiting) in [
            ("1", "自分"),
            ("2", "自分 (Aさんへの相談)"),
            ("3", "自分, Bさん (PR のレビュー)"),
            ("4", "接合テスト環境"),
            ("5", "933"),
            ("6", ""),
            ("7", "me"),
        ] {
            board.tasks.insert(
                id.to_string(),
                Task {
                    waiting: waiting.to_string(),
                    ..Task::default()
                },
            );
        }
        assert!(!board.is_blocked("1") && board.is_your_turn("1"));
        assert!(!board.is_blocked("2") && board.is_your_turn("2"));
        assert!(board.is_blocked("3") && !board.is_your_turn("3"), "外部要因が混ざれば待ち");
        assert!(board.is_blocked("4") && !board.is_your_turn("4"));
        assert!(board.is_blocked("5") && !board.is_your_turn("5"), "解けていない数字札は待ち");
        assert!(!board.is_blocked("6") && !board.is_your_turn("6"), "札なしは着手可");
        assert!(!board.is_blocked("7") && board.is_your_turn("7"), "英語の me も利用者の番");
        assert!(!board.is_blocked("9") && !board.is_your_turn("9"), "知らないIDは着手可扱い");
    }

    /// 小さな一覧。100 は 101・102 と子バンドル 200(201 を持つ)を持つ。300 は素の1行、
    /// 400 は子が全員完了した束、600 は畳まれたルート。
    fn sample() -> Board {
        Board::parse(
            &serde_json::json!({
                "roots": ["100", "300", "400", "600"],
                "deadRoots": ["600"],
                "bundles": ["100", "200", "400", "600"],
                "children": {"100": ["200"], "200": [], "300": [], "400": []},
                "members": {
                    "100": [{"id": "101", "title": "一つ目"}, {"id": "102", "title": "二つ目"}],
                    "200": [{"id": "201", "title": "孫"}],
                    "400": []
                },
                "childOrder": {"100": ["101", "102", "200"], "200": ["201"]},
                "tagLabel": {"100": "親バンドル", "200": "子バンドル"},
                "tasks": {"101": {"title": "一つ目"}},
                "ui": {"model": "opus", "effort": "high"}
            })
            .to_string(),
        )
        .expect("読める")
    }

    /// 並びのファイルで「素タスク → 束 → 素タスク」と指定したら、その順で描く。
    /// 素タスクを先に全部並べる実装だと束が末尾へ回り、作業順と食い違う。
    #[test]
    fn child_order_mixes_plain_tasks_and_bundles() {
        let data = serde_json::json!({
            "roots": ["p"],
            "bundles": ["p", "b"],
            "children": {"p": ["b"]},
            "members": {
                "p": [{"id": "t1", "title": "1a"}, {"id": "t2", "title": "1b"}, {"id": "t4", "title": "4"}],
                "b": [{"id": "t3a", "title": "3a"}]
            },
            "childOrder": {"p": ["t1", "t2", "b", "t4"], "b": ["t3a"]},
            "tasks": {}, "tagLabel": {}, "deadRoots": [], "ui": {}
        });
        let board = Board::parse(&data.to_string()).expect("読める");
        let node = build_node(&board, "p", "", 0, &mut Vec::new());
        let Node::Box { body, .. } = &node else {
            panic!("束は箱になる");
        };
        let ids: Vec<&str> = body.iter().map(Node::id).collect();
        assert_eq!(ids, vec!["t1", "t2", "b", "t4"], "束が末尾へ回っていない");
    }

    /// childOrder に載らない子(完了束の中抜きで繰り上がった子など)も落とさない。
    #[test]
    fn children_missing_from_child_order_still_render() {
        let data = serde_json::json!({
            "roots": ["p"],
            "bundles": ["p"],
            "children": {},
            "members": {"p": [{"id": "t1", "title": "1"}, {"id": "t9", "title": "9"}]},
            "childOrder": {"p": ["t1"]},
            "tasks": {}, "tagLabel": {}, "deadRoots": [], "ui": {}
        });
        let board = Board::parse(&data.to_string()).expect("読める");
        let node = build_node(&board, "p", "", 0, &mut Vec::new());
        let Node::Box { body, .. } = &node else {
            panic!("束は箱になる");
        };
        let ids: Vec<&str> = body.iter().map(Node::id).collect();
        assert_eq!(ids, vec!["t1", "t9"]);
    }

    #[test]
    fn folded_roots_are_not_drawn() {
        let board = sample();
        assert_eq!(board.live_roots(), vec!["100", "300", "400"]);
        let ids: Vec<String> = build_roots(&board).iter().map(|n| n.id().to_string()).collect();
        assert_eq!(ids, vec!["100", "300", "400"]);
        assert_eq!(board.model(), "opus");
        assert_eq!(board.effort(), "high");
    }

    #[test]
    fn a_bundle_whose_children_all_finished_is_still_a_box() {
        let board = sample();
        assert!(!board.is_truly_childless("400"), "bundles に載っていれば箱");
        assert!(board.is_truly_childless("300"), "一度も子を持たない素の1行");
    }

    #[test]
    fn a_childless_root_is_a_plain_row_not_a_box() {
        let board = sample();
        assert!(matches!(
            build_node(&board, "300", "", 0, &mut Vec::new()),
            Node::Row { .. }
        ));
        assert!(matches!(
            build_node(&board, "100", "", 0, &mut Vec::new()),
            Node::Box { .. }
        ));
    }

    #[test]
    fn an_empty_bundle_gets_one_selectable_blank_row() {
        let board = sample();
        let node = build_node(&board, "400", "", 0, &mut Vec::new());
        let Node::Box { body, .. } = &node else {
            panic!("箱のはず: {node:?}")
        };
        assert_eq!(body.len(), 1);
        assert!(matches!(body[0], Node::Empty { .. }));
        assert_eq!(body[0].id(), "400::empty");
        assert!(is_placeholder(body[0].id()));
    }

    #[test]
    fn nesting_keeps_headers_before_their_contents() {
        let board = sample();
        let node = build_node(&board, "100", "", 0, &mut Vec::new());
        let mut flat = Vec::new();
        flatten(std::slice::from_ref(&node), &mut flat);
        assert_eq!(flat, vec!["100", "101", "102", "200", "201"]);
    }

    #[test]
    fn cycles_do_not_hang_the_tree() {
        let mut board = sample();
        board.children.insert("100".into(), vec!["200".into()]);
        board.children.insert("200".into(), vec!["100".into()]);
        let node = build_node(&board, "100", "", 0, &mut Vec::new());
        let mut flat = Vec::new();
        flatten(std::slice::from_ref(&node), &mut flat);
        assert!(flat.len() < 10, "循環で潜り続けている: {flat:?}");
    }

    #[test]
    fn titles_fall_back_through_members_then_labels() {
        let board = sample();
        assert_eq!(board.label_of("101", None), "一つ目");
        assert_eq!(board.label_of("100", None), "親バンドル");
        assert_eq!(board.label_of("999", Some("members の題")), "members の題");
        assert_eq!(board.label_of("999", None), "999", "何も無ければID自身");
    }

    #[test]
    fn broken_payloads_are_rejected_rather_than_guessed() {
        assert!(Board::parse("").is_none());
        assert!(Board::parse("[]").is_none());
        // 欠けたキーは空で埋まる(一覧が部分的に壊れても描ける)
        let thin = Board::parse(r#"{"roots":["a"]}"#).expect("読める");
        assert_eq!(thin.live_roots(), vec!["a"]);
        assert_eq!(thin.label_of("x", None), "x");
    }

    #[test]
    fn members_may_arrive_as_plain_ids() {
        let board = Board::parse(r#"{"members": {"1": ["2", {"id": "3", "title": "題"}]}}"#)
            .expect("読める");
        assert_eq!(board.members["1"][0].id, "2");
        assert_eq!(board.members["1"][1].title, "題");
    }
}
