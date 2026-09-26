//! タスクの md への**書き**。一覧から打つ操作(起票・完了・親替え・並べ替え・起動条件)の口。
//!
//! ## 錠
//!
//! 書き手は複数ありうる(窓の中の糸・タブの中の Claude)ので、**全ての書きが**同じ錠を
//! 取る。錠のファイルは `<tasks>/.create_lock`。待ちは [`LOCK_WAIT`] で打ち切る——死んだ
//! 書き手が錠を握ったまま消えたときに、呼び出しの糸を永久に止めないため。
//!
//! ## 書き方(temp + rename)
//!
//! ファイルは全て [`write_atomic`] を通す——同じディレクトリへ隠し名で書き、`fsync` して
//! から `rename` で被せる。途中で落ちても半分書けた md が残らない(読み手は全部を
//! 舐めるので、壊れた1件が一覧ごと落とす)。

use std::collections::BTreeMap;
use std::io::Write as _;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::lang::Lang;
use crate::vault::{FIELD_ALIASES, Item, build_tree, load_items, state_from_file, state_in_english};

/// 錠を待つ上限。
const LOCK_WAIT: Duration = Duration::from_secs(5);
/// 錠のファイル名。`.md` でないので読みの走査に載らない。
const LOCK_NAME: &str = ".create_lock";
/// 状態欄に置ける値。
const STATES: [&str; 2] = ["進行", "完了"];
/// 起動モデル・effort の候補。
pub const LAUNCH_MODELS: [&str; 3] = ["sonnet", "opus", "fable"];
pub const LAUNCH_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// 書き先ひと揃い。[`crate::vault::Vault`] と同じ場所を指す書き側。
///
/// `Clone` なのは呼び手がスレッドへ持って行くため。中身はパスだけで、状態は一切持たない
/// ——2本同時に走っても、錠と rename が面倒を見る。
#[derive(Clone, Debug)]
pub struct Writer {
    home: PathBuf,
    state_dir: PathBuf,
}

impl Writer {
    /// 稼働時の既定(`paths::tasks_home` / `paths::state`)。
    #[must_use]
    pub fn new() -> Self {
        Self::at(&crate::paths::tasks_home(), &crate::paths::state())
    }

    /// タスクの置き場の親と状態ディレクトリを指して開く。
    #[must_use]
    pub fn at(home: &Path, state_dir: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            state_dir: state_dir.to_path_buf(),
        }
    }

    /// タスク md の置き場。
    #[must_use]
    pub fn tdir(&self) -> PathBuf {
        self.home.join("tasks")
    }

    /// 並びのファイル([`crate::order`])。
    #[must_use]
    pub fn order_file(&self) -> PathBuf {
        crate::order::path(&self.home)
    }

    /// 起動条件(モデル・effort)の保存先。
    #[must_use]
    pub fn launch_file(&self) -> PathBuf {
        self.state_dir.join("board_ui.json")
    }

    /// いまのタスク一式(キャッシュ無しで毎回読む)。
    fn items(&self) -> BTreeMap<String, Item> {
        load_items(&self.tdir())
    }

    fn lock(&self) -> Result<Lock, String> {
        Lock::take(&self.tdir().join(LOCK_NAME))
    }
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- 口 ----------

/// 新しいタスクを置き場の直下(ルート)に1枚書く。ID は置き場の最大+1。返すのは ID。
///
/// # Errors
/// 題が空・錠が取れない・書き出しに失敗したとき。
pub fn create_task(w: &Writer, title: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(crate::tr!("The name is empty", "名前が空です").into());
    }
    let _lock = w.lock()?;
    let tdir = w.tdir();
    std::fs::create_dir_all(&tdir)
        .map_err(|_| crate::tr!("Can't create the task folder", "置き場を作れません").to_string())?;
    let day = today();
    let id = next_id(&tdir);
    let text = new_task_text(&id, "", "進行", &day, title);
    write_atomic(&tdir.join(format!("{id}.md")), &text)
        .map_err(|_| crate::tr!("Couldn't write the file", "書き込みに失敗しました").to_string())?;
    Ok(id)
}

/// 状態(`進行` / `完了`)と更新日を書く。
///
/// # Errors
/// ID か状態が不正・錠が取れない・書き出しに失敗したとき。
pub fn set_state(w: &Writer, id: &str, state: &str) -> Result<(), String> {
    let invalid = || crate::tr!("The ID or status is invalid", "IDまたは状態の指定が不正です").to_string();
    let state = state_from_file(state);
    if !STATES.contains(&state.as_str()) {
        return Err(invalid());
    }
    let _lock = w.lock()?;
    let id = id.trim();
    let f = w.tdir().join(format!("{id}.md"));
    if id.is_empty() || !f.is_file() {
        return Err(invalid());
    }
    let text = std::fs::read_to_string(&f).map_err(|_| invalid())?;
    let text = set_fm_field(&text, "状態", &state).ok_or_else(invalid)?;
    let text = set_fm_field(&text, "更新日", &today()).ok_or_else(invalid)?;
    write_atomic(&f, &text).map_err(|_| invalid())
}

/// 親替え。親欄1行だけを書く。`parent` が空ならルートへ。
///
/// # Errors
/// タスク・移す先が無い・自分の配下へ移そうとした・錠が取れないとき。
pub fn move_task(w: &Writer, id: &str, parent: &str) -> Result<(), String> {
    let id = id.trim();
    let parent = parent.trim();
    let _lock = w.lock()?;
    let items = w.items();
    if !items.contains_key(id) {
        return Err(crate::tr!("Task not found", "タスクが見つかりません").into());
    }
    if !parent.is_empty() {
        if !items.contains_key(parent) {
            return Err(crate::tr!("Destination not found", "移動先が見つかりません").into());
        }
        let (_, _, ancestors_of) = build_tree(&items);
        if parent == id
            || ancestors_of
                .get(parent)
                .is_some_and(|ancestors| ancestors.iter().any(|x| x == id))
        {
            return Err(crate::tr!("Can't move a task under itself", "自分の配下へは移動できません").into());
        }
    }
    if set_parent_field(&w.tdir().join(format!("{id}.md")), parent) {
        Ok(())
    } else {
        Err(crate::tr!("Couldn't write the file", "書き込みに失敗しました").into())
    }
}

/// 兄弟(同じ親の子・ルートどうし)の新しい並びを並びのファイルへ写す([`crate::order`])。
///
/// # Errors
/// 錠が取れない・書き出しに失敗したとき。
pub fn save_order(w: &Writer, siblings: &[String]) -> Result<(), String> {
    let _lock = w.lock()?;
    let file = w.order_file();
    let list = crate::order::with_siblings(&crate::order::load(&file), siblings);
    write_atomic(&file, &crate::order::render(&list))
        .map_err(|error| format!("can't write the task order: {error}"))
}

/// 起動条件(`{"model": …, "effort": …}`)を読む。壊れた値・無い値は既定へ倒す。
#[must_use]
pub fn launch_settings(state_dir: &Path) -> Value {
    let stored = std::fs::read_to_string(state_dir.join("board_ui.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_else(|| json!({}));
    let pick = |key: &str, allowed: &[&str], default: &str| -> String {
        stored
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| allowed.contains(value))
            .unwrap_or(default)
            .to_string()
    };
    json!({
        "model": pick("model", &LAUNCH_MODELS, "sonnet"),
        "effort": pick("effort", &LAUNCH_EFFORTS, "medium"),
    })
}

/// 起動条件を書く。候補に無い値は黙って無視し、いまの値を保つ。返すのは書いた後の値。
///
/// # Errors
/// 書き出しに失敗したとき。
pub fn save_launch_settings(w: &Writer, update: &Value) -> Result<Value, String> {
    let mut current = launch_settings(&w.state_dir);
    for (key, allowed) in [("model", &LAUNCH_MODELS[..]), ("effort", &LAUNCH_EFFORTS[..])] {
        if let Some(value) = update.get(key).and_then(Value::as_str)
            && allowed.contains(&value)
        {
            current[key] = json!(value);
        }
    }
    write_atomic(&w.launch_file(), &current.to_string())
        .map_err(|error| format!("can't save the launch settings: {error}"))?;
    Ok(current)
}

// ---------- 錠 ----------

/// `flock` で取る排他。落とすと外れる。
struct Lock {
    file: std::fs::File,
}

impl Lock {
    /// 取れるまで待つ(上限 [`LOCK_WAIT`])。取れなければ理由を返す。
    fn take(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| format!("can't open the lock: {e}"))?;
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            // SAFETY: 開いたばかりの記述子を渡すだけ。
            let r = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if r == 0 {
                return Ok(Self { file });
            }
            if Instant::now() >= deadline {
                return Err(crate::tr!("Someone else is writing (waited 5 seconds)", "先に書いている人が居る(5秒待った)").to_string());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // SAFETY: 自分が握っている記述子を外すだけ。
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

// ---------- 書き口(temp + rename)----------

/// 同じディレクトリの隠し名へ書いて、`fsync` してから被せる。
///
/// 途中の名前が `.md` で終わらないのが要——読み手(`load_items`)は拡張子で拾うので、
/// 被せる前の半端なファイルを読むことがない。
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let tmp = dir.join(format!(".{name}.tmp.{}.{}", std::process::id(), unique()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// 同じミリ秒に2本書いても名前がぶつからないための通し番号。
fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

// ---------- frontmatter ----------

/// frontmatter の1欄だけを差し替える。無ければ末尾へ足す。
///
/// frontmatter が無い/閉じていないファイルは `None`——本文だけのファイルに
/// 欄を発明しない(呼び出し側が失敗として返す)。
#[must_use]
pub fn set_fm_field(text: &str, field: &str, value: &str) -> Option<String> {
    if !text.starts_with("---") {
        return None;
    }
    let end = find_fm_end(text)?;
    let fm = &text[3..end];
    let mut lines: Vec<String> = fm.split('\n').map(str::to_string).collect();
    let (key, value) = written_field(&lines, field, value);
    let head = format!("{key}:");
    match lines.iter().position(|l| l.starts_with(&head)) {
        Some(i) => lines[i] = format!("{key}: {value}"),
        None => lines.push(format!("{key}: {value}")),
    }
    Some(format!("{}{}{}", &text[..3], lines.join("\n"), &text[end..]))
}

/// 欄の名と値を、ファイルが使っている言語で(`FIELD_ALIASES`)。英語の欄
/// (`status:` など)で書かれたファイルへは英語で書く。どちらの欄も無いファイルは
/// 日本語のまま。
fn written_field(lines: &[String], field: &str, value: &str) -> (String, String) {
    let Some((en, _)) = FIELD_ALIASES.iter().find(|(_, ja)| *ja == field) else {
        return (field.to_string(), value.to_string());
    };
    let has = |key: &str| lines.iter().any(|line| line.starts_with(&format!("{key}:")));
    let english = !has(field)
        && (has(en)
            || FIELD_ALIASES.iter().any(|(other, _)| has(other))
                && !FIELD_ALIASES.iter().any(|(_, ja)| has(ja)));
    if !english {
        let value = if field == "状態" { state_from_file(value) } else { value.to_string() };
        return (field.to_string(), value);
    }
    let value = if field == "状態" { state_in_english(value) } else { value.to_string() };
    ((*en).to_string(), value)
}

/// 新しいタスクの md。欄の名と状態の値は、いまの表示の言語で書く
/// (読む側は `vault::parse_fm` でどちらも読める)。
fn new_task_text(id: &str, parent: &str, state: &str, day: &str, title: &str) -> String {
    match crate::lang::current() {
        Lang::En => format!(
            "---\nid: {id}\nparent: {parent}\nstatus: {}\ncreated: {day}\nupdated: {day}\n---\n\n# {title}\n",
            state_in_english(state)
        ),
        Lang::Ja => format!(
            "---\nID: {id}\n親: {parent}\n状態: {state}\n作成日: {day}\n更新日: {day}\n---\n\n# {title}\n"
        ),
    }
}

/// 閉じの `\n---` の位置。
fn find_fm_end(text: &str) -> Option<usize> {
    text.get(3..)?.find("\n---").map(|i| i + 3)
}

/// 今日。frontmatter の更新日はこれで倒す。
fn today() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d").to_string()
}

/// 親欄1行だけを差し替えて更新日を倒す。空でルート化。
///
/// 親欄が無いファイルには**足さない**——当たらなければ何も起きない。
fn set_parent_field(f: &Path, parent: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(f) else {
        return false;
    };
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?m)^(親|parent):.*$").expect("親欄の型"));
    let text = match re.captures(&text) {
        Some(found) => {
            let whole = found.get(0).expect("the whole match");
            format!(
                "{}{}: {parent}{}",
                &text[..whole.start()],
                &found[1],
                &text[whole.end()..]
            )
        }
        None => text,
    };
    let text = set_fm_field(&text, "更新日", &today()).unwrap_or(text);
    write_atomic(f, &text).is_ok()
}

/// 次のID。置き場(とその下のフォルダ)を舐めて数字名の最大+1。錠の中で呼ぶ。
///
/// 数と見なすのは ASCII 数字だけ。
fn next_id(tdir: &Path) -> String {
    let mut max: u128 = 0;
    let mut stack = vec![tdir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_some_and(|x| x == "md")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                && !stem.is_empty()
                && stem.chars().all(|c| c.is_ascii_digit())
                && let Ok(n) = stem.parse::<u128>()
            {
                max = max.max(n);
            }
        }
    }
    (max + 1).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::DONE;
    use std::collections::BTreeSet;

    /// この回だけの置き場。
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("armature-write-test-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            let me = Self { root };
            std::fs::create_dir_all(me.writer().tdir()).expect("掘れる");
            std::fs::create_dir_all(me.root.join("state")).expect("掘れる");
            me
        }

        fn writer(&self) -> Writer {
            Writer::at(&self.root.join("home"), &self.root.join("state"))
        }

        fn task(&self, id: &str, text: &str) -> &Self {
            std::fs::write(self.writer().tdir().join(format!("{id}.md")), text).expect("書ける");
            self
        }

        fn read(&self, id: &str) -> String {
            std::fs::read_to_string(self.writer().tdir().join(format!("{id}.md"))).expect("読める")
        }
    }

    fn fm(id: &str, parent: &str, state: &str, title: &str) -> String {
        format!("---\nID: {id}\n親: {parent}\n状態: {state}\n作成日: 2026-01-01\n更新日: 2026-01-01\n---\n\n# {title}\n")
    }

    #[test]
    fn setting_a_state_also_stamps_the_day() {
        let s = Scratch::new("state");
        s.task("101", &fm("101", "", "進行", "題"));
        set_state(&s.writer(), "101", DONE).expect("書ける");
        let text = s.read("101");
        assert!(text.contains("状態: 完了"), "{text}");
        assert!(text.contains(&format!("更新日: {}", today())), "{text}");
        assert!(text.contains("作成日: 2026-01-01"), "他の欄は動かない: {text}");
    }

    #[test]
    fn a_state_outside_the_two_values_is_refused() {
        let s = Scratch::new("state-bad");
        s.task("101", &fm("101", "", "進行", "題"));
        assert!(set_state(&s.writer(), "101", "保留").is_err());
        assert!(s.read("101").contains("状態: 進行"), "断ったら書かない");
        assert!(set_state(&s.writer(), "404", DONE).is_err(), "無いタスクは断る");
    }

    #[test]
    fn a_new_task_takes_the_next_number() {
        let _ja = crate::lang::scoped(crate::lang::Lang::Ja);
        let s = Scratch::new("create");
        s.task("101", &fm("101", "", "進行", "既存"));
        let id = create_task(&s.writer(), "新しい仕事").expect("書ける");
        assert_eq!(id, "102", "既存の最大+1");
        let text = s.read("102");
        assert!(text.contains("# 新しい仕事"), "{text}");
        assert!(text.contains("親: \n"), "ルートに置く: {text}");
        assert!(create_task(&s.writer(), "  ").is_err(), "空の題は断る");
    }

    #[test]
    fn a_file_written_in_english_keeps_its_english_keys() {
        let text = "---\nid: 5\nparent: \nstatus: active\nupdated: 2026-01-01\n---\n\n# Task\n";
        let done = set_fm_field(text, "状態", DONE).expect("frontmatter");
        assert!(done.contains("status: done"), "{done}");
        assert!(!done.contains("状態"), "{done}");
        let dated = set_fm_field(&done, "更新日", "2026-09-24").expect("frontmatter");
        assert!(dated.contains("updated: 2026-09-24"), "{dated}");
        let japanese = "---\nID: 5\n状態: 進行\n---\n";
        let done = set_fm_field(japanese, "状態", "done").expect("frontmatter");
        assert!(done.contains("状態: 完了"), "{done}");
    }

    #[test]
    fn new_tasks_are_written_in_the_display_language() {
        let s = Scratch::new("create-english");
        {
            let _en = crate::lang::scoped(crate::lang::Lang::En);
            create_task(&s.writer(), "First").expect("書ける");
        }
        let text = s.read("1");
        assert!(text.contains("id: 1\nparent: \nstatus: active\n"), "{text}");
        let (fm, _) = crate::vault::parse_fm(&text);
        assert_eq!(fm["状態"], "進行");
    }

    #[test]
    fn moving_writes_only_the_parent_line() {
        let s = Scratch::new("move");
        s.task("101", &fm("101", "", "進行", "親"));
        s.task("102", &fm("102", "", "進行", "子になる"));
        move_task(&s.writer(), "102", "101").expect("移せる");
        let text = s.read("102");
        assert!(text.contains("親: 101"), "{text}");
        assert!(text.contains("# 子になる"), "{text}");
        move_task(&s.writer(), "102", "").expect("ルートへ戻せる");
        assert!(s.read("102").contains("親: \n"));
    }

    #[test]
    fn moving_a_task_into_its_own_subtree_is_refused() {
        let _ja = crate::lang::scoped(crate::lang::Lang::Ja);
        let s = Scratch::new("move-cycle");
        s.task("101", &fm("101", "", "進行", "親"));
        s.task("102", &fm("102", "101", "進行", "子"));
        assert_eq!(
            move_task(&s.writer(), "101", "102"),
            Err("自分の配下へは移動できません".into())
        );
        assert_eq!(
            move_task(&s.writer(), "101", "999"),
            Err("移動先が見つかりません".into())
        );
        assert!(s.read("101").contains("親: \n"), "断ったら書かない");
    }

    #[test]
    fn a_sibling_order_lands_in_the_order_file() {
        let s = Scratch::new("order");
        let w = s.writer();
        std::fs::write(w.order_file(), "3\n7\n9\n5\n").expect("書ける");
        save_order(&w, &["5".into(), "7".into()]).expect("書ける");
        assert_eq!(
            std::fs::read_to_string(w.order_file()).expect("読める"),
            "3\n5\n7\n9\n"
        );
    }

    #[test]
    fn the_launch_settings_merge_and_drop_nonsense() {
        let s = Scratch::new("launch");
        let w = s.writer();
        let got = save_launch_settings(&w, &json!({"model": "opus", "effort": "そこそこ"}))
            .expect("書ける");
        assert_eq!(got["model"], "opus");
        assert_eq!(got["effort"], "medium", "知らない値は既定のまま");
        // 次の呼び出しで前回の選択が残っていること(保存されている)。
        let got = save_launch_settings(&w, &json!({"effort": "high"})).expect("書ける");
        assert_eq!(got["model"], "opus");
        assert_eq!(got["effort"], "high");
        assert_eq!(launch_settings(&s.root.join("state")), got);
    }

    #[test]
    fn broken_launch_settings_fall_back_to_the_defaults() {
        let s = Scratch::new("launch-broken");
        std::fs::write(s.writer().launch_file(), r#"{"model": "gpt", "effort": "high"}"#)
            .expect("書ける");
        let got = launch_settings(&s.root.join("state"));
        assert_eq!(got["model"], "sonnet");
        assert_eq!(got["effort"], "high", "実在する値はそのまま");
    }

    #[test]
    fn the_lock_is_released_when_it_falls_out_of_scope() {
        let s = Scratch::new("lock");
        let path = s.writer().tdir().join(LOCK_NAME);
        {
            let _held = Lock::take(&path).expect("取れる");
        }
        Lock::take(&path).expect("落ちたら外れている");
    }

    #[test]
    fn writers_at_once_never_hand_out_the_same_id() {
        // 錠が効いていることの本番。採番と書き込みが1本ずつ直列になる。
        //
        // 数えるのは**通った回だけ**。混んだ機体では fsync が伸びて [`LOCK_WAIT`] に
        // 当たる回があるため——見たい性質(同じIDを2人に渡さない・書いたファイルが
        // 上書きで消えない)はこの数え方で十分に出る。
        let s = Scratch::new("race");
        let w = s.writer();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let w = w.clone();
                std::thread::spawn(move || create_task(&w, &format!("同時{i}")).unwrap_or_default())
            })
            .collect();
        let handed: Vec<String> = handles
            .into_iter()
            .map(|h| h.join().expect("落ちない"))
            .filter(|id| !id.is_empty())
            .collect();
        let unique: BTreeSet<&String> = handed.iter().collect();
        assert!(handed.len() >= 2, "全部が錠待ちで落ちたら検分にならない: {handed:?}");
        assert_eq!(unique.len(), handed.len(), "同じIDを2人に渡していない: {handed:?}");
        let files = std::fs::read_dir(w.tdir())
            .expect("読める")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
            .count();
        assert_eq!(files, handed.len(), "上書きで消えたファイルが無い");
    }

    #[test]
    fn a_write_leaves_no_half_file_behind() {
        let s = Scratch::new("atomic");
        let f = s.writer().tdir().join("101.md");
        write_atomic(&f, "本文\n").expect("書ける");
        assert_eq!(std::fs::read_to_string(&f).expect("読める"), "本文\n");
        let leftovers: Vec<String> = std::fs::read_dir(s.writer().tdir())
            .expect("読める")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "仮の名前が残らない: {leftovers:?}");
    }

    #[test]
    fn a_field_is_appended_when_it_is_not_there_yet() {
        let text = "---\nID: 1\n---\n\n# 題\n";
        let got = set_fm_field(text, "待ち", "7").expect("frontmatterがある");
        assert_eq!(got, "---\nID: 1\n待ち: 7\n---\n\n# 題\n");
        assert!(set_fm_field("# 題だけ\n", "状態", "完了").is_none(), "本文だけには足さない");
    }
}
