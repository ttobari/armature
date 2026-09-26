//! タスクの置き場を読む。`<タスクの置き場の親>/tasks/*.md` と並びのファイル(`order.txt`)
//! を読んで、一覧([`crate::board::Board`])が読む形の JSON を組む。
//!
//! ## frontmatter は YAML ではない
//!
//! 素朴な「キー: 値」1行1件で、`---` の間だけを見る。YAML パーサは入れない——YAML の
//! 型推論(`ID: 0123` は数か字か)を持ち込むと静かに形が変わる。
//!
//! ## 木の組み方
//!
//! ```text
//!   items(md)  →  build_tree(親ポインタ)  →  並び(order.txt)
//!                    ↓                          ↓
//!                 children_of ────────→  兄弟を並びどおりに → members
//!                    ↓                          ↓
//!                  is_bundle(この時点の親)    alive(完了で畳む・deadRoots)
//!                    ↓                          ↓
//!                 ui_children(子バンドルだけ) → 完了バンドルの中抜き
//! ```
//!
//! 順序が要る所が2つある。`is_bundle` は `alive` の**前**に採る(子が全員完了した
//! 束を束のまま残すため)。`ui_children` は `alive` の**後**の `children_of` から組む。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

/// 状態欄の完了値。
pub const DONE: &str = "完了";

/// 英語でも書ける frontmatter の欄(英語, 日本語)。読むときは日本語の名へ寄せる
/// ——`status: done` は `状態: 完了` と同じ。書くときはファイルが使っている方に合わせる
/// (`write::set_fm_field`)。新しく作るファイルは、いまの表示の言語で書く。
pub const FIELD_ALIASES: [(&str, &str); 5] = [
    ("id", "ID"),
    ("parent", "親"),
    ("status", "状態"),
    ("created", "作成日"),
    ("updated", "更新日"),
];

/// 英語でも書ける状態の値(英語, 日本語)。
pub const STATE_ALIASES: [(&str, &str); 2] = [("active", "進行"), ("done", "完了")];

/// 状態の値を日本語の名へ寄せる(`done` → `完了`)。知らない値はそのまま。
#[must_use]
pub fn state_from_file(value: &str) -> String {
    let value = value.trim();
    STATE_ALIASES
        .iter()
        .find(|(en, _)| en.eq_ignore_ascii_case(value))
        .map_or_else(|| value.to_string(), |(_, ja)| (*ja).to_string())
}

/// 状態の値を英語の名へ(`完了` → `done`)。英語で書かれたファイルへ書くときに使う。
#[must_use]
pub fn state_in_english(value: &str) -> String {
    let value = value.trim();
    STATE_ALIASES
        .iter()
        .find(|(_, ja)| *ja == value)
        .map_or_else(|| value.to_string(), |(en, _)| (*en).to_string())
}

/// タスク1件(一覧を組むのに要る欄だけ)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub id: String,
    pub state: String,
    /// 所属先のID。ルート(無所属)は空。
    pub parent: String,
    pub title: String,
    /// frontmatter の `待ち` 欄(生の値)。空なら着手を止めているものは無い。
    /// 解けた待ちを落とすのは [`open_waiting`] の仕事で、ここは書いてあるまま持つ。
    pub waiting: String,
}

/// バンドル直属のタスク1件(`members` の要素)。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Member {
    id: String,
    title: String,
    state: String,
}

/// 読み先ひと揃い。
pub struct Vault {
    tdir: PathBuf,
    order_file: PathBuf,
    state_dir: PathBuf,
    /// 前回読んだ md 群(ディレクトリの署名が変わるまで使い回す)。
    cache: Option<(Signature, BTreeMap<String, Item>)>,
}

/// `tdir` 配下の md の名前と更新時刻。
type Signature = Vec<(String, u128)>;

/// 親ポインタから組んだ木ひと揃い(ルートの並び・ID→子・ID→祖先)。
pub type Tree = (
    Vec<String>,
    BTreeMap<String, Vec<String>>,
    BTreeMap<String, Vec<String>>,
);

impl Vault {
    /// 稼働時の既定(`paths::tasks_home` / `paths::state`)。
    #[must_use]
    pub fn new() -> Self {
        Self::at(&crate::paths::tasks_home(), &crate::paths::state())
    }

    /// タスクの置き場の親と状態ディレクトリを指して開く。
    #[must_use]
    pub fn at(home: &Path, state_dir: &Path) -> Self {
        Self {
            tdir: home.join("tasks"),
            order_file: crate::order::path(home),
            state_dir: state_dir.to_path_buf(),
            cache: None,
        }
    }

    /// 一覧が読む形の JSON を組む。
    pub fn data(&mut self) -> serde_json::Value {
        let items = self.items();
        let (mut roots, mut children_of, _) = build_tree(&items);
        // 兄弟を並びどおりに。並びに無いものは ID の順で後ろへ。
        let rank = crate::order::rank(&crate::order::load(&self.order_file));
        crate::order::sort(&mut roots, &rank);
        for kids in children_of.values_mut() {
            crate::order::sort(kids, &rank);
        }

        let mut members = build_members(&items, &children_of);
        // 束かどうかは `alive` の剪定**前**に確定させる——子が全員完了した束を
        // 素の1行へ落とさないため。
        let is_bundle: HashSet<String> = children_of
            .iter()
            .filter(|(_, kids)| !kids.is_empty())
            .map(|(t, _)| t.clone())
            .collect();

        let dead_roots: Vec<String> = roots
            .iter()
            .filter(|r| !alive(r, &mut children_of, &items, &members, &mut HashSet::new()))
            .cloned()
            .collect();

        // 一覧へ渡す children は「子を持つ子」だけ——素通しすると葉タスクが
        // 行と箱の両方で二重に描かれる。
        let mut ui_children: BTreeMap<String, Vec<String>> = children_of
            .iter()
            .map(|(t, kids)| {
                (
                    t.clone(),
                    kids.iter().filter(|c| is_bundle.contains(*c)).cloned().collect(),
                )
            })
            .collect();
        let mut visited = HashSet::new();
        for r in &roots {
            elide_done_bundles(r, &mut ui_children, &mut members, &is_bundle, &items, &mut visited);
        }

        let tasks: serde_json::Map<String, serde_json::Value> = items
            .values()
            .map(|it| {
                (
                    it.id.clone(),
                    serde_json::json!({
                        "title": it.title,
                        "state": it.state,
                        "waiting": open_waiting(&it.waiting, &items),
                    }),
                )
            })
            .collect();
        let tag_label: serde_json::Map<String, serde_json::Value> = items
            .values()
            .map(|it| (it.id.clone(), serde_json::json!(it.title)))
            .collect();
        let members_json: serde_json::Map<String, serde_json::Value> = members
            .iter()
            .map(|(k, list)| {
                let list: Vec<serde_json::Value> = list
                    .iter()
                    .map(|m| serde_json::json!({"id": m.id, "title": m.title, "state": m.state}))
                    .collect();
                (k.clone(), serde_json::Value::Array(list))
            })
            .collect();
        let mut bundles: Vec<String> = is_bundle.into_iter().collect();
        bundles.sort();

        serde_json::json!({
            "roots": roots,
            "deadRoots": dead_roots,
            "bundles": bundles,
            "children": ui_children,
            "members": members_json,
            // 素タスクと子バンドルを分けて渡すと両者の相対順が失われる。
            // 描画側が順を復元するための1本。
            "childOrder": children_of,
            "tagLabel": tag_label,
            "tasks": tasks,
            "ui": crate::write::launch_settings(&self.state_dir),
        })
    }

    /// md 群を読む(前回から1件も動いていなければ読み直さない)。
    fn items(&mut self) -> BTreeMap<String, Item> {
        let sig = self.signature();
        if let Some((cached, items)) = &self.cache
            && cached == &sig
            && !sig.is_empty()
        {
            return items.clone();
        }
        let items = load_items(&self.tdir);
        self.cache = Some((sig, items.clone()));
        items
    }

    /// ディレクトリの中身(名前と更新時刻)。
    fn signature(&self) -> Signature {
        let mut entries: Vec<(String, u128)> = std::fs::read_dir(&self.tdir)
            .map(|dir| {
                dir.flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
                    .map(|e| {
                        let mtime = e
                            .metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map_or(0, |d| d.as_nanos());
                        (e.file_name().to_string_lossy().to_string(), mtime)
                    })
                    .collect()
            })
            .unwrap_or_default();
        entries.sort();
        entries
    }
}

impl Default for Vault {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- md を読む ----------

/// 先頭の `---` … `---` を「キー: 値」で読む。戻りは (frontmatter, 本文)。
///
/// YAML ではない(モジュール冒頭参照)。`:` を含まない行は捨て、最初の `:` だけで割る。
#[must_use]
pub fn parse_fm(text: &str) -> (BTreeMap<String, String>, &str) {
    let mut fm = BTreeMap::new();
    if !text.starts_with("---") {
        return (fm, text);
    }
    let Some(offset) = text.get(3..).and_then(|rest| rest.find("\n---")) else {
        return (fm, text);
    };
    let end = 3 + offset;
    for line in text[3..end].lines() {
        if let Some((k, v)) = line.split_once(':') {
            fm.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    // 英語の欄は日本語の名へ寄せる(`FIELD_ALIASES`)。両方あれば日本語が勝つ。
    for (en, ja) in FIELD_ALIASES {
        if !fm.contains_key(ja)
            && let Some(value) = fm.get(en).cloned()
        {
            fm.insert(ja.to_string(), value);
        }
    }
    if let Some(state) = fm.get_mut("状態") {
        *state = state_from_file(state);
    }
    (fm, text.get(end + 4..).unwrap_or(""))
}

/// 本文の最初の H1(`# 題`)。無ければ空。
#[must_use]
pub fn h1_title(body: &str) -> String {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?m)^#\s+(.+)$").expect("H1 の型は正しい"));
    re.captures(body)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_default()
}

/// ID の数値部分。数字が無ければ 0。
#[must_use]
pub fn idnum(tid: &str) -> u128 {
    let digits: String = tid.chars().filter(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

/// `待ち` 欄から、**まだ解けていない待ちだけ**を残す。
///
/// 札はカンマ(半角・全角どちらも)で区切る。**数字だけの札はタスクIDの待ち**と
/// みなし、そのタスクが完了へ倒れた時点で自動で外す——解けた待ちを手で消す作業を
/// 残すと、消し忘れた行が着手可から沈んだまま埋もれる。数字以外の札
/// は誰も倒せないので、書いた本人が消すまで残る。
///
/// 戻りが空文字なら着手可。
fn open_waiting(raw: &str, items: &BTreeMap<String, Item>) -> String {
    let open: Vec<&str> = raw
        .split([',', '、'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .filter(|token| {
            let cleared = token.chars().all(|c| c.is_ascii_digit())
                && items.get(*token).is_some_and(|it| it.state == DONE);
            !cleared
        })
        .collect();
    open.join(", ")
}

/// ID の既定の並び(数値部分→文字列)。
fn id_key(tid: &str) -> (u128, String) {
    (idnum(tid), tid.to_string())
}

/// `tdir/*.md` を全部読む。
#[must_use]
pub fn load_items(tdir: &Path) -> BTreeMap<String, Item> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(tdir)
        .map(|dir| {
            dir.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|p| id_key(&p.file_stem().unwrap_or_default().to_string_lossy()));

    let mut items = BTreeMap::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let (fm, body) = parse_fm(&text);
        let stem = f.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let tid = fm
            .get("ID")
            .filter(|v| !v.is_empty())
            .cloned()
            .unwrap_or(stem)
            .trim()
            .to_string();
        let title = h1_title(body);
        items.insert(
            tid.clone(),
            Item {
                state: fm.get("状態").cloned().unwrap_or_default(),
                parent: fm.get("親").cloned().unwrap_or_default(),
                title: if title.is_empty() {
                    crate::tr!(format!("Task {tid}"), format!("タスク {tid}"))
                } else {
                    title
                },
                waiting: fm.get("待ち").cloned().unwrap_or_default().trim().to_string(),
                id: tid,
            },
        );
    }
    items
}

/// 親ポインタから親子木を組む。戻りは (ルート, 子, 祖先)。子とルートは ID の順。
///
/// バンドルかどうかを示す欄は無い——「誰かに親として指されているID」が動的に
/// 束になるだけで、その判定は呼び出し側の責務。ここは辺を機械的に組む。
#[must_use]
pub fn build_tree(items: &BTreeMap<String, Item>) -> Tree {
    let mut children_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (tid, it) in items {
        if !it.parent.is_empty() {
            children_of.entry(it.parent.clone()).or_default().push(tid.clone());
        }
    }
    for kids in children_of.values_mut() {
        kids.sort_by_key(|t| id_key(t));
    }

    let ancestors_of: BTreeMap<String, Vec<String>> = items
        .keys()
        .map(|tid| {
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            let mut cur = items.get(tid);
            while let Some(it) = cur {
                if it.parent.is_empty() || !seen.insert(it.parent.clone()) {
                    break;
                }
                out.push(it.parent.clone());
                cur = items.get(&it.parent);
            }
            (tid.clone(), out)
        })
        .collect();

    let mut roots: Vec<String> = items
        .iter()
        .filter(|(_, it)| it.parent.is_empty())
        .map(|(tid, _)| tid.clone())
        .collect();
    roots.sort_by_key(|t| id_key(t));
    (roots, children_of, ancestors_of)
}

/// 親ID → 直属メンバー(子を持たない子だけ・完了は除く)。並びは `children_of` の順。
fn build_members(
    items: &BTreeMap<String, Item>,
    children_of: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<Member>> {
    children_of
        .iter()
        .map(|(parent, kids)| {
            let members = kids
                .iter()
                .filter_map(|k| items.get(k))
                .filter(|it| it.state != DONE)
                .filter(|it| children_of.get(&it.id).is_none_or(Vec::is_empty))
                .map(|it| Member {
                    id: it.id.clone(),
                    title: it.title.clone(),
                    state: it.state.clone(),
                })
                .collect();
            (parent.clone(), members)
        })
        .collect()
}

/// 完了で畳めるか。生きた子だけを残すよう `children_of` を書き換える。
///
/// 状態だけ見て節を描き続けると、束を完了にしても画面から消えない。**訪れた節
/// すべてに**子リストを書き戻す——葉タスクにも空の子リストができる。
///
/// 循環(異常データ)では自分自身を数えないよう打ち切る。
fn alive(
    t: &str,
    children_of: &mut BTreeMap<String, Vec<String>>,
    items: &BTreeMap<String, Item>,
    members: &BTreeMap<String, Vec<Member>>,
    on_path: &mut HashSet<String>,
) -> bool {
    if !on_path.insert(t.to_string()) {
        return false;
    }
    let mut kids = Vec::new();
    for c in children_of.get(t).cloned().unwrap_or_default() {
        if alive(&c, children_of, items, members, on_path) {
            kids.push(c);
        }
    }
    on_path.remove(t);
    let live = match items.get(t) {
        None => false, // 実在しないID(データ異常への防御)は生存扱いしない
        Some(it) => {
            it.state != DONE
                || members.get(t).is_some_and(|m| !m.is_empty())
                || !kids.is_empty()
        }
    };
    children_of.insert(t.to_string(), kids);
    live
}

/// 完了バンドルの中抜き。
///
/// 深いほうから畳み、生きた子バンドルと直属メンバーを親の直下へ昇格させる。
fn elide_done_bundles(
    t: &str,
    ui_children: &mut BTreeMap<String, Vec<String>>,
    members: &mut BTreeMap<String, Vec<Member>>,
    is_bundle: &HashSet<String>,
    items: &BTreeMap<String, Item>,
    visited: &mut HashSet<String>,
) {
    if !visited.insert(t.to_string()) {
        return;
    }
    for k in ui_children.get(t).cloned().unwrap_or_default() {
        elide_done_bundles(&k, ui_children, members, is_bundle, items, visited);
    }
    let mut new_kids = Vec::new();
    for k in ui_children.get(t).cloned().unwrap_or_default() {
        let done_bundle =
            is_bundle.contains(&k) && items.get(&k).is_some_and(|it| it.state == DONE);
        if done_bundle {
            new_kids.extend(ui_children.get(&k).cloned().unwrap_or_default());
            let moved = members.get(&k).cloned().unwrap_or_default();
            members.entry(t.to_string()).or_default().extend(moved);
            ui_children.remove(&k);
            members.remove(&k);
        } else {
            new_kids.push(k);
        }
    }
    ui_children.insert(t.to_string(), new_kids);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// この回だけの置き場(一時ディレクトリ)。
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("armature-vault-test-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("tasks")).expect("作れる");
            std::fs::create_dir_all(root.join("state")).expect("作れる");
            Self { root }
        }

        fn task(&self, id: &str, fm: &str, title: &str) -> &Self {
            std::fs::write(
                self.root.join("tasks").join(format!("{id}.md")),
                format!("---\nID: {id}\n{fm}---\n\n# {title}\n"),
            )
            .expect("書ける");
            self
        }

        fn order(&self, text: &str) -> &Self {
            std::fs::write(crate::order::path(&self.root), text).expect("書ける");
            self
        }

        fn vault(&self) -> Vault {
            Vault::at(&self.root, &self.root.join("state"))
        }

        fn data(&self) -> serde_json::Value {
            self.vault().data()
        }
    }

    #[test]
    fn a_waiting_on_a_finished_task_clears_itself() {
        // `待ち` にタスクIDを書いた行は、相手が完了へ倒れた時点で自動で解ける
        // ——解けた待ちを手で消す作業を残すと、消し忘れた行が着手可から沈んだまま
        // 埋もれる。数字以外の札は誰も倒せないので残る。
        let s = Scratch::new("waiting");
        s.task("101", "親: \n状態: 進行\n待ち: 102\n", "相手待ち")
            .task("102", "親: \n状態: 完了\n", "もう終わった相手")
            .task("103", "親: \n状態: 進行\n待ち: 104, 自分\n", "二重待ち")
            .task("104", "親: \n状態: 進行\n", "まだ動いている相手")
            .task("105", "親: \n状態: 進行\n", "待ちの無い行");
        let data = s.data();
        let waiting = |id: &str| data["tasks"][id]["waiting"].as_str().unwrap_or("").to_string();
        assert_eq!(waiting("101"), "", "相手が完了なら待ちは解ける");
        assert_eq!(waiting("103"), "104, 自分", "解けていない札だけ残る");
        assert_eq!(waiting("105"), "", "欄が無ければ着手可");
    }

    #[test]
    fn frontmatter_is_read_as_plain_key_value_not_yaml() {
        let (fm, body) = parse_fm("---\nID: 12\n親:\n状態: 進行\n---\n\n# 題\n本文\n");
        assert_eq!(fm["ID"], "12");
        assert_eq!(fm["親"], "", "ルートは欄だけあって空");
        assert_eq!(fm["状態"], "進行");
        assert_eq!(h1_title(body), "題");
    }

    #[test]
    fn a_body_without_frontmatter_is_left_alone() {
        let (fm, body) = parse_fm("# 題だけ\n");
        assert!(fm.is_empty());
        assert_eq!(body, "# 題だけ\n");
        // 閉じない frontmatter も本文として素通しする。
        let (fm, body) = parse_fm("---\nID: 3\n");
        assert!(fm.is_empty());
        assert_eq!(body, "---\nID: 3\n");
    }

    #[test]
    fn colons_in_the_value_stay_in_the_value() {
        let (fm, _) = parse_fm("---\n成果物: https://example.com/a\n---\n\n");
        assert_eq!(fm["成果物"], "https://example.com/a");
    }

    #[test]
    fn ids_sort_by_their_number_not_by_their_text() {
        assert_eq!(idnum("100"), 100);
        assert_eq!(idnum("task-7"), 7);
        assert_eq!(idnum("なし"), 0);
        let mut ids = vec!["100".to_string(), "9".to_string(), "10".to_string()];
        ids.sort_by_key(|t| id_key(t));
        assert_eq!(ids, vec!["9", "10", "100"]);
    }

    #[test]
    fn the_tree_comes_from_parent_pointers_alone() {
        let s = Scratch::new("tree");
        s.task("1", "親:\n状態: 進行\n", "ルート")
            .task("2", "親: 1\n状態: 進行\n", "子")
            .task("3", "親: 2\n状態: 進行\n", "孫");
        let d = s.data();
        assert_eq!(d["roots"], serde_json::json!(["1"]));
        assert_eq!(d["bundles"], serde_json::json!(["1", "2"]), "指されていれば束");
        assert_eq!(d["children"]["1"], serde_json::json!(["2"]));
        assert_eq!(d["members"]["2"][0]["id"], "3");
        assert!(
            d["children"].get("3").is_some(),
            "葉にも空の子リストが要る(欄数がずれる)"
        );
    }

    #[test]
    fn a_bundle_whose_children_all_finished_stays_a_bundle() {
                let s = Scratch::new("dead-kids");
        s.task("1", "親:\n状態: 進行\n", "束")
            .task("2", "親: 1\n状態: 完了\n", "終わった子");
        let d = s.data();
        assert_eq!(d["bundles"], serde_json::json!(["1"]));
        assert_eq!(d["children"]["1"], serde_json::json!([]));
        assert_eq!(d["members"]["1"], serde_json::json!([]));
        assert_eq!(d["deadRoots"], serde_json::json!([]), "親が進行なら生きている");
    }

    #[test]
    fn a_root_whose_everything_is_finished_is_folded_away() {
        let s = Scratch::new("dead-root");
        s.task("1", "親:\n状態: 完了\n", "終わった束")
            .task("2", "親: 1\n状態: 完了\n", "終わった子")
            .task("5", "親:\n状態: 進行\n", "生きたルート");
        let d = s.data();
        assert_eq!(d["roots"], serde_json::json!(["1", "5"]), "roots からは抜かない");
        assert_eq!(d["deadRoots"], serde_json::json!(["1"]));
    }

    #[test]
    fn a_finished_bundle_in_the_middle_is_elided() {
                let s = Scratch::new("elide");
        s.task("1", "親:\n状態: 進行\n", "ルート")
            .task("2", "親: 1\n状態: 完了\n", "完了した中間の束")
            .task("3", "親: 2\n状態: 進行\n", "生きた孫束")
            .task("4", "親: 3\n状態: 進行\n", "ひ孫")
            .task("5", "親: 2\n状態: 進行\n", "生きた直属");
        let d = s.data();
        assert_eq!(d["children"]["1"], serde_json::json!(["3"]), "孫が親の直下へ昇る");
        assert!(d["children"].get("2").is_none(), "中抜きされた束は消える");
        assert_eq!(d["members"]["1"][0]["id"], "5", "直属も昇る");
    }

    #[test]
    fn english_keys_are_read_as_the_japanese_ones() {
        let (fm, body) = parse_fm("---\nid: 7\nparent: 3\nstatus: done\ncreated: 2026-09-24\n---\n\n# Hi\n");
        assert_eq!(fm["ID"], "7");
        assert_eq!(fm["親"], "3");
        assert_eq!(fm["状態"], DONE);
        assert_eq!(fm["作成日"], "2026-09-24");
        assert!(body.contains("# Hi"));
        let (fm, _) = parse_fm("---\n状態: 進行\nstatus: done\n---\n");
        assert_eq!(fm["状態"], "進行", "両方あれば日本語が勝つ");
        let (fm, _) = parse_fm("---\n状態: Active\n---\n");
        assert_eq!(fm["状態"], "進行", "値だけ英語でも寄せる");
    }

    #[test]
    fn a_task_without_a_heading_falls_back_to_its_id() {
        let _ja = crate::lang::scoped(crate::lang::Lang::Ja);
        let s = Scratch::new("no-title");
        std::fs::write(
            s.root.join("tasks").join("7.md"),
            "---\nID: 7\n親:\n状態: 進行\n---\n\n本文だけ\n",
        )
        .expect("書ける");
        assert_eq!(s.data()["tasks"]["7"]["title"], "タスク 7");
    }

    #[test]
    fn a_cycle_in_the_data_does_not_hang_the_read() {
        // 異常データ。再帰で潜り続けず、打ち切る。
        let s = Scratch::new("cycle");
        s.task("1", "親: 2\n状態: 進行\n", "い")
            .task("2", "親: 1\n状態: 進行\n", "ろ");
        let d = s.data();
        assert_eq!(d["roots"], serde_json::json!([]), "循環はルートを持たない");
    }

    #[test]
    fn reading_twice_gives_the_same_answer() {
        let s = Scratch::new("cache");
        s.task("1", "親:\n状態: 進行\n", "い");
        let mut vault = s.vault();
        let first = vault.data();
        let second = vault.data();
        assert_eq!(first["tasks"], second["tasks"], "署名が同じなら読み直さない");
        s.task("1", "親:\n状態: 進行\n", "い(改)");
        // 署名(更新時刻)が変わるまで待たずに済むよう、キャッシュを持たない口で読む。
        assert_eq!(s.data()["tasks"]["1"]["title"], "い(改)");
    }

    #[test]
    fn the_order_file_decides_the_order_of_roots_and_children() {
        let s = Scratch::new("order");
        s.task("1", "親:\n状態: 進行\n", "束")
            .task("2", "親: 1\n状態: 進行\n", "あ")
            .task("3", "親: 1\n状態: 進行\n", "い")
            .task("4", "親: 1\n状態: 進行\n", "う")
            .task("5", "親:\n状態: 進行\n", "もう1つのルート")
            .task("6", "親:\n状態: 進行\n", "並びに無いルート")
            .order("5\n4\n1\n2\n");
        let d = s.data();
        assert_eq!(d["roots"], serde_json::json!(["5", "1", "6"]), "載っていないものは ID の順で後ろ");
        let ids: Vec<&str> = d["members"]["1"]
            .as_array()
            .expect("配列")
            .iter()
            .filter_map(|m| m["id"].as_str())
            .collect();
        assert_eq!(ids, ["4", "2", "3"]);
    }

    #[test]
    fn without_an_order_file_everything_is_in_id_order() {
        let s = Scratch::new("no-order");
        s.task("10", "親:\n状態: 進行\n", "十")
            .task("9", "親:\n状態: 進行\n", "九");
        assert_eq!(s.data()["roots"], serde_json::json!(["9", "10"]));
    }

    #[test]
    fn the_launch_settings_ride_along() {
        let s = Scratch::new("ui");
        s.task("1", "親:\n状態: 進行\n", "い");
        let ui = s.data()["ui"].clone();
        assert_eq!(ui["model"], "sonnet");
        assert_eq!(ui["effort"], "medium");
    }
}
