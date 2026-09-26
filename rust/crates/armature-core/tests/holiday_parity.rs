//! 祝日のゴールデン突合。Python の jpholiday(1.0.3)と2020-01-01〜2035-12-31 の
//! **全日**を突き合わせる。
//!
//! フィクスチャの作り直し:
//!
//! ```text
//! uv run --no-project --with jpholiday python3 rust/scripts/dump_holidays.py \
//!     > rust/crates/armature-core/tests/fixtures/holidays.tsv
//! ```
//!
//! 落ちたときに疑うのは Rust 側(`src/holiday.rs`)。フィクスチャを取り直して
//! 通すのは、正本を書き換えて辻褄を合わせる操作なのでやらない——ただし
//! 祝日法そのものが変わった(新しい特例が公布された)ときだけは、取り直しが
//! 正しい直し方になる。

use std::collections::BTreeMap;

use armature_core::holiday::{holiday_name, is_holiday};
use jiff::civil::{Date, date};

const FIXTURE: &str = include_str!("fixtures/holidays.tsv");

fn expected() -> BTreeMap<Date, String> {
    FIXTURE
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let (iso, name) = line.split_once('\t').expect("TSVの列が足りない");
            let day: Date = iso.parse().expect("日付として読めない");
            (day, name.to_string())
        })
        .collect()
}

const START: Date = date(2020, 1, 1);
const END: Date = date(2035, 12, 31);

#[test]
fn matches_python_jpholiday_for_every_day() {
    let table = expected();
    let mut day = START;
    let mut checked = 0u32;
    while day <= END {
        let got = holiday_name(day);
        let want = table.get(&day).cloned();
        assert_eq!(
            got, want,
            "{day} の祝日判定がPython版と違う\n  Rust  : {got:?}\n  Python: {want:?}"
        );
        // 名前と真偽が食い違わないことも一緒に見る(暦の色付けは is_holiday を使う)。
        assert_eq!(is_holiday(day), want.is_some(), "{day}");
        checked += 1;
        day = day.tomorrow().expect("翌日");
    }
    assert_eq!(checked, 5844, "突き合わせた日数が想定と違う");
}

#[test]
fn fixture_covers_the_hard_cases() {
    // 痩せたフィクスチャで通っても何も保証しない——数年に一度しか現れない形が
    // 入っていることを、フィクスチャ自身に対して確かめる。
    let table = expected();
    assert_eq!(table.len(), 286, "祝日の件数が想定と違う");
    // 五輪特例(2020・2021の移動)
    assert_eq!(table.get(&date(2020, 7, 23)).unwrap(), "海の日");
    assert_eq!(table.get(&date(2021, 8, 8)).unwrap(), "山の日");
    // 連休をまたぐ振替休日
    assert_eq!(table.get(&date(2020, 5, 6)).unwrap(), "憲法記念日 振替休日");
    // 国民の休日(2回だけ現れる)
    let national = table.values().filter(|v| *v == "国民の休日").count();
    assert_eq!(national, 2, "国民の休日の件数");
    // 振替休日そのもの
    let substitutes = table.values().filter(|v| v.ends_with("振替休日")).count();
    assert!(substitutes >= 20, "振替休日が {substitutes} 件しかない");
}
