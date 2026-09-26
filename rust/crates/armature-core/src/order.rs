//! タスクの並び。タスクの置き場の親に置く `order.txt` に、**上から順に ID を1行ずつ**書く。
//!
//! ```text
//!   ~/Armature/
//!     order.txt      ← 12 / 7 / 15 / …(1行1つ)
//!     tasks/*.md
//! ```
//!
//! 並びは全タスクで1本。兄弟どうし(同じ親を持つタスク・ルートどうし)の前後は、この
//! 1本の中の位置で決まる。載っていないタスクは、載っているものの後ろに ID の順で並ぶ
//! ——ファイルが無ければ全部が ID の順。手でも Claude にでも書き換えられる形にしてある
//! (空行と `#` の行は読み飛ばす)。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 並びのファイルの名。
pub const FILE_NAME: &str = "order.txt";

/// タスクの置き場の親(`paths::tasks_home`)から並びのファイルの在り処。
#[must_use]
pub fn path(tasks_home: &Path) -> PathBuf {
    tasks_home.join(FILE_NAME)
}

/// 並びを読む。無い・読めないなら空(全部 ID の順)。同じ ID は最初の1回だけ数える。
#[must_use]
pub fn load(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|text| parse(&text))
        .unwrap_or_default()
}

fn parse(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|id| seen.insert((*id).to_string()))
        .map(str::to_string)
        .collect()
}

/// ID → 並びの中の位置。
#[must_use]
pub fn rank(list: &[String]) -> HashMap<String, usize> {
    list.iter()
        .enumerate()
        .map(|(index, id)| (id.clone(), index))
        .collect()
}

/// 兄弟の列を並びどおりに並べる。載っていないものは後ろに ID の順(数の部分→文字列)。
pub fn sort(ids: &mut [String], rank: &HashMap<String, usize>) {
    ids.sort_by(|a, b| {
        let key = |id: &String| {
            (
                rank.get(id).copied().unwrap_or(usize::MAX),
                crate::vault::idnum(id),
                id.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
}

/// 兄弟の新しい並び(`siblings`)を全体の並びへ写す。
///
/// 兄弟をいったん全部抜き、**抜いた中でいちばん前に居た位置**へ新しい順で差し戻す。
/// ほかのタスクの位置は動かさない。兄弟が1つも載っていなければ末尾へ足す。
#[must_use]
pub fn with_siblings(list: &[String], siblings: &[String]) -> Vec<String> {
    let first = list
        .iter()
        .position(|id| siblings.contains(id))
        .unwrap_or(list.len());
    let before = list[..first].iter().filter(|id| !siblings.contains(id));
    let after = list[first..].iter().filter(|id| !siblings.contains(id));
    before
        .chain(siblings.iter())
        .chain(after)
        .cloned()
        .collect()
}

/// ファイルに書く形。
#[must_use]
pub fn render(list: &[String]) -> String {
    let mut text = list.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn blank_lines_comments_and_repeats_are_skipped() {
        assert_eq!(parse("# top\n12\n\n 7 \n12\n"), ids(&["12", "7"]));
    }

    #[test]
    fn listed_ids_come_first_and_the_rest_follow_by_number() {
        let rank = rank(&ids(&["30", "2"]));
        let mut kids = ids(&["10", "2", "9", "30"]);
        sort(&mut kids, &rank);
        assert_eq!(kids, ids(&["30", "2", "9", "10"]));
    }

    #[test]
    fn a_new_sibling_order_goes_where_the_first_sibling_was() {
        // 5 と 7 は兄弟。3 と 9 は別の親の子——位置を動かさない。
        let list = ids(&["3", "7", "9", "5"]);
        assert_eq!(
            with_siblings(&list, &ids(&["5", "7"])),
            ids(&["3", "5", "7", "9"])
        );
        // 載っていなかった兄弟は末尾へ。
        assert_eq!(
            with_siblings(&list, &ids(&["1", "2"])),
            ids(&["3", "7", "9", "5", "1", "2"])
        );
        assert_eq!(with_siblings(&[], &ids(&["1"])), ids(&["1"]));
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let list = ids(&["4", "1", "12"]);
        assert_eq!(parse(&render(&list)), list);
    }
}
