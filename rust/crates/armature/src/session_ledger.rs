//! 起こした Claude タブの台帳。閉じたタブを戻すために持つ。
//!
//! 起動のたびに session id を**こちらで振って** `claude --session-id` へ渡し、
//! ここへ控える。閉じたタブから id を逆算せずに済むのが要点——会話ログを漁って
//! 「どれが閉じた分か」を当てる作業が丸ごと要らなくなる。
//!
//! 控えはファイルなので、窓を焼き直しても残る。

use std::path::PathBuf;

/// 閉じた控えを何件まで持つか。戻す相手は普通いちばん新しい1件で、
/// 際限なく溜めても読み込みが重くなるだけ。
const CLOSED_LIMIT: usize = 40;

/// 控えの1件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// `--session-id` に渡した uuid。戻すときは `--resume` へ同じものを渡す。
    pub id: String,
    /// タブの題。戻すときも同じ題で立てる。
    pub tab: String,
    /// 起こした場所。戻すときも同じ場所で立てる。
    pub cwd: String,
    /// 閉じたと分かった刻(**ミリ秒**)。生きているうちは `None`。
    ///
    /// 秒で刻んでいた頃は、同じ秒に閉じた2枚が同じ数になって並び順が決まらず、
    /// ⌘⇧T が「2個ぐらい前に閉じたタブ」を戻していた。
    pub closed_at: Option<u64>,
}

impl Entry {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_string(),
            tab: value.get("tab")?.as_str().unwrap_or_default().to_string(),
            cwd: value.get("cwd")?.as_str().unwrap_or_default().to_string(),
            closed_at: value.get("closed_at").and_then(serde_json::Value::as_u64),
        })
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "tab": self.tab,
            "cwd": self.cwd,
            "closed_at": self.closed_at,
        })
    }
}

fn path() -> PathBuf {
    armature_core::geo::state_dir().join("cockpit-sessions.json")
}

/// いまの刻をミリ秒で。**秒では足りない**——同じ秒に2枚閉じると並び順が消える。
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| {
            u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
        })
}

fn load() -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    value
        .as_array()
        .map(|rows| rows.iter().filter_map(Entry::from_json).collect())
        .unwrap_or_default()
}

/// 書き替えの最中を読ませない。tmp へ書いてから置き換える。
fn save(entries: &[Entry]) {
    let file = path();
    let Some(dir) = file.parent() else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let rows: Vec<serde_json::Value> = entries.iter().map(Entry::to_json).collect();
    let Ok(text) = serde_json::to_string(&serde_json::Value::Array(rows)) else {
        return;
    };
    let tmp = file.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
}

/// 起こしたタブを控える。
pub fn record_opened(id: &str, tab: &str, cwd: &str) {
    let mut entries = load();
    entries.retain(|entry| entry.id != id);
    entries.push(Entry {
        id: id.to_string(),
        tab: tab.to_string(),
        cwd: cwd.to_string(),
        closed_at: None,
    });
    trim(&mut entries);
    save(&entries);
}

/// 生きている id に無い控えへ閉じた刻を刻む。刻んだ件数を返す。
pub fn sweep(live: &[String]) -> usize {
    let mut entries = load();
    let closed = sweep_entries(&mut entries, live, now());
    if closed > 0 {
        trim(&mut entries);
        save(&entries);
    }
    closed
}

/// 閉じた瞬間を控えへ刻む。**閉じる側から呼ぶ**のが本筋。
///
/// [`sweep`] は「押した時点で消えていたもの」へまとめて同じ刻を打つので、複数枚を
/// 続けて閉じるとどれが最後か分からなくなる。⌘W で閉じるときは瞬間が分かるので、
/// ここで1枚だけ正確に刻む。
/// すでに刻んである控えは触らない。
pub fn mark_closed(id: &str) {
    let mut entries = load();
    let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.id == id && entry.closed_at.is_none())
    else {
        return;
    };
    entry.closed_at = Some(now());
    trim(&mut entries);
    save(&entries);
}

/// 戻す相手。閉じた刻がいちばん新しいもの。
///
/// 同じ刻で並んだときは**台帳の後ろにあるほう**を採る——後から起こしたタブが
/// 後ろに積まれるので、それが最後に触っていた1枚である見込みが高い。
/// `max_by_key` は同点のとき最後の要素を返すので、その性質をそのまま使う。
#[must_use]
pub fn latest_closed() -> Option<Entry> {
    load()
        .into_iter()
        .filter(|entry| entry.closed_at.is_some())
        .max_by_key(|entry| entry.closed_at.unwrap_or(0))
}

/// 生きている id に無いものへ刻を刻む。すでに刻んである分は触らない。
fn sweep_entries(entries: &mut [Entry], live: &[String], now: u64) -> usize {
    let mut closed = 0;
    for entry in entries.iter_mut() {
        if entry.closed_at.is_some() || live.iter().any(|id| id == &entry.id) {
            continue;
        }
        entry.closed_at = Some(now);
        closed += 1;
    }
    closed
}

/// 閉じた控えだけを古い順に落とす。生きている分は数に入れない。
fn trim(entries: &mut Vec<Entry>) {
    let mut closed: Vec<u64> = entries.iter().filter_map(|entry| entry.closed_at).collect();
    if closed.len() <= CLOSED_LIMIT {
        return;
    }
    closed.sort_unstable();
    let cutoff = closed[closed.len() - CLOSED_LIMIT];
    entries.retain(|entry| entry.closed_at.is_none_or(|at| at >= cutoff));
}

#[cfg(test)]
mod tests {
    use super::{Entry, sweep_entries, trim};

    /// 続けて2枚閉じたら、刻は必ず違う数になること。
    ///
    /// 秒で刻んでいた頃は同じ数になり、⌘⇧T が戻す相手を取り違えていた。
    #[test]
    fn two_closes_in_the_same_second_do_not_collide() {
        let first = super::now();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = super::now();
        assert!(second > first, "刻の分解能が粗い: {first} と {second}");
        // ミリ秒であること(秒なら 2026年でも 10^10 に届かない)。
        assert!(first > 1_700_000_000_000, "秒のまま刻んでいる: {first}");
    }

    fn entry(id: &str, closed_at: Option<u64>) -> Entry {
        Entry {
            id: id.to_string(),
            tab: format!("tab {id}"),
            cwd: "/Users/x/dev".to_string(),
            closed_at,
        }
    }

    #[test]
    fn a_tab_that_is_gone_gets_a_closing_time() {
        let mut entries = vec![entry("alive", None), entry("gone", None)];
        assert_eq!(sweep_entries(&mut entries, &["alive".to_string()], 100), 1);
        assert_eq!(entries[0].closed_at, None, "生きている分に刻を打っている");
        assert_eq!(entries[1].closed_at, Some(100));
    }

    #[test]
    fn an_already_closed_tab_keeps_its_first_closing_time() {
        // 2度目の掃きで刻を上書きすると、戻す順(新しい順)が狂う。
        let mut entries = vec![entry("gone", Some(100))];
        assert_eq!(sweep_entries(&mut entries, &[], 200), 0);
        assert_eq!(entries[0].closed_at, Some(100));
    }

    #[test]
    fn trimming_drops_the_oldest_closed_and_keeps_the_living() {
        let mut entries = vec![entry("alive", None)];
        for i in 0..super::CLOSED_LIMIT + 5 {
            entries.push(entry(&format!("closed{i}"), Some(i as u64)));
        }
        trim(&mut entries);
        assert!(
            entries.iter().any(|entry| entry.id == "alive"),
            "生きている控えを落としている"
        );
        assert_eq!(
            entries.iter().filter(|e| e.closed_at.is_some()).count(),
            super::CLOSED_LIMIT
        );
        assert!(
            !entries.iter().any(|entry| entry.id == "closed0"),
            "古い方から落ちていない"
        );
    }
}
