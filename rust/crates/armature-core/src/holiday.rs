//! 日本の祝日。Python版 `boardtui.clock.holiday_name`(jpholiday 1.0.3)の代わり。
//!
//! ## なぜ自前で持つのか
//!
//! 計画では jpholiday crate 0.2.0 を引く予定だったが、あちらは rustc 1.96 以上を
//! 要求する(この機体の stable は 1.94.1・2026-08-11実測)。1つ前の 0.1.4 は
//! 2020年6月で更新が止まっており、同年の五輪特例(海の日・スポーツの日・山の日の
//! 移動)より前の版——盤が描く暦の範囲でいきなり外れる。ツールチェーンを上げる
//! のは他の作業を巻き込むので、祝日法の条文をここへ写した。
//!
//! 写した以上、写し間違いは目視では見つからない。担保は
//! `tests/holiday_parity.rs` ——2020-01-01〜2035-12-31 の全日を Python の
//! jpholiday と突き合わせる(フィクスチャは `scripts/dump_holidays.py`)。
//!
//! ## 効く範囲
//!
//! 仮定: **2020年以降だけ**を扱い、それより前は祝日なしとして返す。2019年以前は
//! 天皇誕生日が12/23、スポーツの日が体育の日で、条文ごと別物になる——盤の暦が
//! 描くのは常に今月なので、過去の名前を持っていても使い道がない。ここで嘘の
//! 名前を返すより、色が付かないほうが害が小さい。
//!
//! 春分・秋分は近似式(有効域1980〜2099)。2020〜2035の16年ぶんは Python 側と
//! 完全に一致することを確認済み。

use jiff::civil::{Date, Weekday};

/// この実装が効き始める年(それより前は常に `None`)。
pub const FIRST_YEAR: i16 = 2020;

/// その日が祝日なら名前。振替休日は「元の名前 + 半角空白 + 振替休日」
/// (jpholiday の書式に合わせてある——盤の暦は名前を出さないが、
/// 突合の相手が名前で答える以上ここも名前で持つ)。
#[must_use]
pub fn holiday_name(day: Date) -> Option<String> {
    if day.year() < FIRST_YEAR {
        return None;
    }
    if let Some(name) = statutory(day) {
        return Some(name.to_string());
    }
    if let Some(origin) = substitute_origin(day) {
        return Some(format!("{origin} 振替休日"));
    }
    if is_national_holiday(day) {
        return Some("国民の休日".to_string());
    }
    None
}

/// 祝日かどうかだけを見る(暦の色付けはこちらで足りる)。
#[must_use]
pub fn is_holiday(day: Date) -> bool {
    if day.year() < FIRST_YEAR {
        return false;
    }
    statutory(day).is_some() || substitute_origin(day).is_some() || is_national_holiday(day)
}

/// 法定の祝日(振替休日・国民の休日を含まない)。
fn statutory(day: Date) -> Option<&'static str> {
    let (y, m, d) = (day.year(), day.month(), day.day());
    match (m, d) {
        (1, 1) => return Some("元日"),
        (2, 11) => return Some("建国記念の日"),
        (2, 23) => return Some("天皇誕生日"),
        (4, 29) => return Some("昭和の日"),
        (5, 3) => return Some("憲法記念日"),
        (5, 4) => return Some("みどりの日"),
        (5, 5) => return Some("こどもの日"),
        (11, 3) => return Some("文化の日"),
        (11, 23) => return Some("勤労感謝の日"),
        _ => {}
    }
    // 山の日。2020は五輪の開会式前日へ、2021は閉会式当日へ動いた。
    if (m, d) == (8, yama_no_hi_day(y)) {
        return Some("山の日");
    }
    if Some(day) == umi_no_hi(y) {
        return Some("海の日");
    }
    if Some(day) == sports_day(y) {
        return Some("スポーツの日");
    }
    if nth_monday(y, 1, 2) == Some(day) {
        return Some("成人の日");
    }
    if nth_monday(y, 9, 3) == Some(day) {
        return Some("敬老の日");
    }
    if (m, d) == (3, equinox_day(y, EQUINOX_SPRING)) {
        return Some("春分の日");
    }
    if (m, d) == (9, equinox_day(y, EQUINOX_AUTUMN)) {
        return Some("秋分の日");
    }
    None
}

/// 振替休日なら、振替元(日曜に当たった祝日)の名前。
///
/// 条文は「日曜と重なったら、その後いちばん近い祝日でない日を休みにする」。
/// 連休の中で重なると1日ずれ込む——2020年の憲法記念日(5/3が日曜)は
/// みどりの日・こどもの日を飛び越えて5/6が振替休日になった。
fn substitute_origin(day: Date) -> Option<&'static str> {
    if day.weekday() == Weekday::Sunday || statutory(day).is_some() {
        return None;
    }
    // 直前の日曜まで遡り、その間が祝日で埋まっているかを見る。
    let mut cur = day.yesterday().ok()?;
    loop {
        let name = statutory(cur)?; // 祝日が途切れたら振替ではない
        if cur.weekday() == Weekday::Sunday {
            return Some(name);
        }
        cur = cur.yesterday().ok()?;
    }
}

/// 国民の休日か——祝日に前後を挟まれた平日(2026-09-22 の敬老の日と秋分の日の間など)。
///
/// 挟む側は法定の祝日だけを数える(振替休日は「国民の祝日」ではない)。日曜は
/// 除く——休みが増えないので条文が対象にしていない。
fn is_national_holiday(day: Date) -> bool {
    if day.weekday() == Weekday::Sunday || statutory(day).is_some() {
        return false;
    }
    let (Ok(prev), Ok(next)) = (day.yesterday(), day.tomorrow()) else {
        return false;
    };
    statutory(prev).is_some() && statutory(next).is_some()
}

/// 山の日の日付(8月の何日か)。
fn yama_no_hi_day(year: i16) -> i8 {
    match year {
        2020 => 10, // 五輪の閉会式翌日へ移動
        2021 => 8,  // 閉会式当日へ移動
        _ => 11,
    }
}

/// 海の日。既定は7月第3月曜、2020・2021は五輪特例。
fn umi_no_hi(year: i16) -> Option<Date> {
    match year {
        2020 => Date::new(2020, 7, 23).ok(),
        2021 => Date::new(2021, 7, 22).ok(),
        _ => nth_monday(year, 7, 3),
    }
}

/// スポーツの日。既定は10月第2月曜、2020・2021は五輪特例(開会式当日)。
fn sports_day(year: i16) -> Option<Date> {
    match year {
        2020 => Date::new(2020, 7, 24).ok(),
        2021 => Date::new(2021, 7, 23).ok(),
        _ => nth_monday(year, 10, 2),
    }
}

/// その月の第 `nth` 月曜。
fn nth_monday(year: i16, month: i8, nth: i8) -> Option<Date> {
    Date::new(year, month, 1)
        .ok()?
        .nth_weekday_of_month(nth, Weekday::Monday)
        .ok()
}

/// 春分の近似式の定数項。
const EQUINOX_SPRING: f64 = 20.8431;
/// 秋分の近似式の定数項。
const EQUINOX_AUTUMN: f64 = 23.2488;

/// 春分・秋分の日(その月の何日か)。
///
/// 官報の公示は前年2月なので厳密には計算で出せない値だが、1980〜2099は
/// この近似式で公示と一致することが知られており、Python の jpholiday も
/// 同じ式を使っている(2020〜2035の全年で一致を確認済み)。
fn equinox_day(year: i16, base: f64) -> i8 {
    let years = f64::from(year) - 1980.0;
    let leaps = f64::from((year - 1980) / 4);
    #[allow(clippy::cast_possible_truncation)] // 結果は必ず19〜24の範囲
    let day = (base + 0.242194 * years - leaps).floor() as i8;
    day
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn fixed_days_have_their_names() {
        assert_eq!(holiday_name(date(2026, 1, 1)).as_deref(), Some("元日"));
        assert_eq!(
            holiday_name(date(2026, 2, 23)).as_deref(),
            Some("天皇誕生日")
        );
        assert_eq!(holiday_name(date(2026, 8, 11)).as_deref(), Some("山の日"));
        assert!(holiday_name(date(2026, 8, 12)).is_none());
    }

    #[test]
    fn happy_mondays_land_on_mondays() {
        // 成人の日=1月第2月曜・敬老の日=9月第3月曜・スポーツの日=10月第2月曜
        for day in [
            date(2026, 1, 12),
            date(2026, 9, 21),
            date(2026, 10, 12),
            date(2026, 7, 20),
        ] {
            assert_eq!(day.weekday(), Weekday::Monday, "{day}");
            assert!(is_holiday(day), "{day}");
        }
    }

    #[test]
    fn olympic_years_moved_three_holidays() {
        assert_eq!(holiday_name(date(2020, 7, 23)).as_deref(), Some("海の日"));
        assert_eq!(
            holiday_name(date(2020, 7, 24)).as_deref(),
            Some("スポーツの日")
        );
        assert_eq!(holiday_name(date(2020, 8, 10)).as_deref(), Some("山の日"));
        // 通常年の位置には立たない
        assert!(holiday_name(date(2020, 8, 11)).is_none());
        assert_eq!(holiday_name(date(2021, 8, 8)).as_deref(), Some("山の日"));
    }

    #[test]
    fn substitute_skips_over_a_run_of_holidays() {
        // 2020-05-03(日)憲法記念日 → 4日・5日は祝日なので振替は6日
        assert_eq!(
            holiday_name(date(2020, 5, 6)).as_deref(),
            Some("憲法記念日 振替休日")
        );
        assert!(holiday_name(date(2020, 5, 7)).is_none());
    }

    #[test]
    fn national_holiday_sits_between_two_holidays() {
        // 2026-09-21 敬老の日(月)・09-23 秋分の日(水)に挟まれた火曜
        assert_eq!(
            holiday_name(date(2026, 9, 22)).as_deref(),
            Some("国民の休日")
        );
    }

    #[test]
    fn before_2020_is_left_alone() {
        // 2019-12-23 は当時の天皇誕生日だが、この実装の効く範囲の外。
        assert!(holiday_name(date(2019, 12, 23)).is_none());
        assert!(!is_holiday(date(2019, 1, 1)));
    }

    #[test]
    fn equinox_formula_matches_the_published_days() {
        assert_eq!(equinox_day(2026, EQUINOX_SPRING), 20);
        assert_eq!(equinox_day(2026, EQUINOX_AUTUMN), 23);
        assert_eq!(equinox_day(2020, EQUINOX_SPRING), 20);
        assert_eq!(equinox_day(2020, EQUINOX_AUTUMN), 22);
    }
}
