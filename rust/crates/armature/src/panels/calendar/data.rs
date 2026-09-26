use std::collections::BTreeMap;
use armature_core::tr;
use std::path::{Path, PathBuf};

use jiff::civil::Date;

const EMBEDDED_MARKS: &str = include_str!("../../../assets/calendar-marks.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkStyle {
    Text,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkColor {
    /// 旧保存データ互換。新しく付ける印には使わない(日曜と同系色になるため)。
    Red,
    Gold,
    /// 旧保存データ互換。新しく付ける印には使わない(土曜と同色になるため)。
    Blue,
    Green,
    Mauve,
}

impl MarkColor {
    /// 新しく付ける印の3色。週末の赤・青は選択肢へ戻さない。
    pub const MENU: [Self; 3] = [Self::Gold, Self::Green, Self::Mauve];

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            // red / blue は既存 calendar-marks.json を読み続けるために残す。
            "red" => Some(Self::Red),
            "gold" => Some(Self::Gold),
            "blue" => Some(Self::Blue),
            "green" => Some(Self::Green),
            "mauve" => Some(Self::Mauve),
            _ => None,
        }
    }

    const fn code(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Gold => "gold",
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Mauve => "mauve",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub style: MarkStyle,
    pub color: MarkColor,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    values: BTreeMap<Date, Mark>,
}

impl Marks {
    #[must_use]
    pub fn load() -> Self {
        std::fs::read_to_string(marks_path())
            .ok()
            .and_then(|source| Self::parse(&source).ok())
            .or_else(|| Self::parse(EMBEDDED_MARKS).ok())
            .unwrap_or_default()
    }

    pub fn parse(source: &str) -> Result<Self, String> {
        let root: serde_json::Value = serde_json::from_str(source)
            .map_err(|error| format!("calendar marks JSON: {error}"))?;
        let object = root["marks"]
            .as_object()
            .ok_or_else(|| "calendar marks JSON has no marks object".to_string())?;
        let mut values = BTreeMap::new();
        for (raw_date, value) in object {
            let date = raw_date
                .parse::<Date>()
                .map_err(|error| format!("bad calendar mark date ({raw_date}): {error}"))?;
            let style = match value["style"].as_str() {
                Some("text") => MarkStyle::Text,
                Some("underline") => MarkStyle::Underline,
                _ => return Err(format!("bad mark style on {raw_date}")),
            };
            let color = value["color"]
                .as_str()
                .and_then(MarkColor::parse)
                .ok_or_else(|| format!("bad mark color on {raw_date}"))?;
            values.insert(date, Mark { style, color });
        }
        Ok(Self { values })
    }

    #[must_use]
    pub fn get(&self, date: Date) -> Option<Mark> {
        self.values.get(&date).copied()
    }

    pub fn set(&mut self, date: Date, mark: Option<Mark>) {
        if let Some(mark) = mark {
            self.values.insert(date, mark);
        } else {
            self.values.remove(&date);
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = marks_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|error| format!("can't create {}: {error}", dir.display()))?;
        }
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut object = serde_json::Map::new();
        for (date, mark) in &self.values {
            object.insert(
                date.to_string(),
                serde_json::json!({
                    "style": match mark.style {
                        MarkStyle::Text => "text",
                        MarkStyle::Underline => "underline",
                    },
                    "color": mark.color.code(),
                }),
            );
        }
        let mut source = serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "marks": object,
        }))
        .map_err(|error| format!("can't encode the calendar marks: {error}"))?;
        source.push('\n');
        let temp = path.with_file_name(format!(".calendar-marks.json.tmp-{}", std::process::id()));
        std::fs::write(&temp, source)
            .map_err(|error| format!("can't write marks to {}: {error}", temp.display()))?;
        std::fs::rename(&temp, path).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            format!("can't replace {}: {error}", path.display())
        })
    }
}

/// 暦の印の置き場(状態の置き場の中)。
fn marks_path() -> PathBuf {
    armature_core::geo::state_dir().join("calendar-marks.json")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Month {
    pub year: i16,
    pub month: i8,
    pub first_weekday: usize,
    pub days: i8,
}

impl Month {
    /// この月を描くのに要る週の行数。月初の曜日ぶん空けてから日数を並べ、7で切り上げる。
    ///
    /// **6で決め打ってはいけない。**5週で収まる月(2026-09〜11など)では最下段が
    /// まるごと空の行になり、暦の下に空白が1行残る。
    #[must_use]
    pub fn weeks(&self) -> usize {
        (self.first_weekday + self.days as usize).div_ceil(7)
    }
}

#[must_use]
// 月名は2026-08-22に暦のカードから外した。戻す日のために残してある。
#[allow(dead_code)]
pub fn month_label(month: i8) -> Option<&'static str> {
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    usize::try_from(month - 1)
        .ok()
        .and_then(|index| MONTHS.get(index).copied())
}

/// 今月から `count` か月ぶん。枚数はパネルの広さで決まるので呼ぶ側が渡す。
#[must_use]
pub fn months_from(today: Date, count: usize) -> Vec<Month> {
    let mut year = today.year();
    let mut month = today.month();
    (0..count)
        .map(|_| {
            let first = Date::new(year, month, 1).expect("the first of a month is a valid date");
            let out = Month {
                year,
                month,
                first_weekday: first.weekday().to_sunday_zero_offset() as usize,
                days: first.last_of_month().day(),
            };
            month += 1;
            if month == 13 {
                year += 1;
                month = 1;
            }
            out
        })
        .collect()
}

/// 暦の見せ方。**戻せるようにしておく**——3か月のグリッドは 2026-09-06 まで
/// 唯一の姿で、暦の中では情報量がいちばん多い。
/// 4週を出すときの週数。
pub const WEEKS_AHEAD: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// 4週を**横の行**で。上に曜日の頭文字、1週が1行。**これが既定**
    /// 。
    #[default]
    FourWeekRows,
    /// 4週を**縦の列**で。左から今週・来週・再来週・4週目、列の中は日〜土の縦並び。
    /// 一度これを既定にしたが、は横の行4本のことだった。戻せるよう残す。
    FourWeekColumns,
    /// 今週と来週の2行(2026-09-06 の最初の形)。
    TwoWeekRows,
    /// 3か月のグリッド。
    ThreeMonths,
}

impl Layout {
    /// 出す週の数。3か月のときは週で数えないので 0。
    #[must_use]
    pub const fn weeks(self) -> usize {
        match self {
            Self::FourWeekColumns | Self::FourWeekRows => WEEKS_AHEAD,
            Self::TwoWeekRows => 2,
            Self::ThreeMonths => 0,
        }
    }
}

fn conf_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let _ = home;
    Some(armature_core::geo::state_dir().join("calendar.conf"))
}

pub const DEFAULT_CONF: &str = "\
# Calendar panel settings. Lines are `key = value`; anything after # is a comment.
# Restart the window after editing (⇧⌘R).

# How the calendar is laid out.
#   4weeks-rows    … four weeks as rows (weekdays on top, one week per row). Default
#   4weeks-columns … four weeks as columns (this week, next week, and the two after)
#   2weeks         … just this week and next week
#   3months        … three month grids
view = 4weeks-rows
";

/// 設定の雛形を用意する。**有る物は書き換えない。**
pub fn prepare() {
    let Some(path) = conf_path() else {
        return;
    };
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, DEFAULT_CONF);
}

/// 設定を読む。読めない・知らない値は既定(weekly)へ倒す。
#[must_use]
#[allow(dead_code)] // 常設は2週固定。旧レイアウトの読み取り口は互換用に残す。
pub fn layout() -> Layout {
    conf_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map_or_else(Layout::default, |source| parse_layout(&source))
}

fn parse_layout(source: &str) -> Layout {
    let mut layout = Layout::default();
    for line in source.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !matches!(key.trim(), "表示" | "view" | "layout") {
            continue;
        }
        layout = match value.trim().to_ascii_lowercase().as_str() {
            "3months" | "months" | "3ヶ月" | "3か月" => Layout::ThreeMonths,
            "4weeks-columns" | "4weeks_columns" | "4週列" => Layout::FourWeekColumns,
            "2weeks" | "2週" => Layout::TwoWeekRows,
            // **`weekly` は4週の行**へ倒す。前に配った conf に
            // `weekly` や `4weeks-columns` と書いてあっても、新しい既定の姿で立つ。
            "4weeks-rows" | "4weeks_rows" | "4週行" | "weekly" | "week" | "週" => {
                Layout::FourWeekRows
            }
            // 打ち間違いで暦が消えないよう、知らない値は既定のまま。
            _ => layout,
        };
    }
    layout
}

/// 今週から `count` 週ぶん、日曜始まりで返す。
///
/// **月をまたいでも切らない。**3か月のグリッドは月の器で切っていたが、週の暦で
/// 月末に切ると「来週の頭が見えない」——先を見るためのパネルなので、日付をそのまま繋げる。
#[must_use]
pub fn weeks_from(today: Date, count: usize) -> Vec<[Date; 7]> {
    let Some(sunday) = week_start(today) else {
        return Vec::new();
    };
    (0..count)
        .filter_map(|week| {
            let mut days = [sunday; 7];
            for (index, slot) in days.iter_mut().enumerate() {
                let offset = i64::try_from(week * 7 + index).ok()?;
                *slot = shift(sunday, offset)?;
            }
            Some(days)
        })
        .collect()
}

/// その日を含む週の日曜。
#[must_use]
pub fn week_start(today: Date) -> Option<Date> {
    shift(today, -i64::from(today.weekday().to_sunday_zero_offset()))
}

fn shift(date: Date, days: i64) -> Option<Date> {
    date.checked_add(jiff::Span::new().days(days)).ok()
}

/// 曜日の頭文字(日曜始まり)。
/// 曜日の見出し(日曜始まり)。日本語は漢字1字、英語は頭文字。
#[must_use]
pub fn weekday_labels() -> [&'static str; 7] {
    tr!(
        ["S", "M", "T", "W", "T", "F", "S"],
        ["日", "月", "火", "水", "木", "金", "土"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 週は日曜から始まり月をまたいでも切れない() {
        // 2026-09-06 は日曜。
        let sunday = Date::new(2026, 9, 6).unwrap();
        assert_eq!(week_start(sunday), Some(sunday));
        // 週の途中から呼んでも同じ日曜へ寄る。
        assert_eq!(week_start(Date::new(2026, 9, 9).unwrap()), Some(sunday));

        let weeks = weeks_from(sunday, 2);
        assert_eq!(weeks.len(), 2);
        assert_eq!(weeks[0][0], sunday);
        assert_eq!(weeks[0][6], Date::new(2026, 9, 12).unwrap());
        assert_eq!(weeks[1][0], Date::new(2026, 9, 13).unwrap());

        // 月末をまたぐ週も7日そのまま繋がる。
        let weeks = weeks_from(Date::new(2026, 9, 29).unwrap(), 2);
        assert_eq!(weeks[0][0], Date::new(2026, 9, 27).unwrap());
        assert_eq!(weeks[0][6], Date::new(2026, 10, 3).unwrap());
        assert_eq!(weeks[1][6], Date::new(2026, 10, 10).unwrap());
    }

    #[test]
    fn 見せ方の設定を読む() {
        assert_eq!(parse_layout(""), Layout::FourWeekRows);
        assert_eq!(parse_layout("表示 = 3months\n"), Layout::ThreeMonths);
        assert_eq!(parse_layout("表示 = 4weeks-columns\n"), Layout::FourWeekColumns);
        assert_eq!(parse_layout("表示 = 2weeks\n"), Layout::TwoWeekRows);
        assert_eq!(parse_layout("view = 4weeks-rows\n"), Layout::FourWeekRows);
        assert_eq!(parse_layout("# 表示 = 3months\n"), Layout::FourWeekRows);
        // 打ち間違いで暦が消えないよう、知らない値は既定のまま。
        assert_eq!(parse_layout("表示 = よくわからない\n"), Layout::FourWeekRows);
        // 雛形はそのまま読める。
        assert_eq!(parse_layout(DEFAULT_CONF), Layout::FourWeekRows);
    }

    /// 前に配った conf には `表示 = weekly`(初回)や `4weeks-columns`(2回目)と
    /// 書いてある。**`weekly` は新しい既定=4週の行へ倒す**——で意味が変わった。
    /// `4weeks-columns` と明示してある機体だけは、書いたとおり縦の列のまま。
    #[test]
    fn 古いweeklyは四週の行へ倒れる() {
        assert_eq!(parse_layout("表示 = weekly\n"), Layout::FourWeekRows);
        assert_eq!(parse_layout("表示 = 4weeks-columns\n"), Layout::FourWeekColumns);
        assert_eq!(Layout::FourWeekRows.weeks(), 4);
        assert_eq!(Layout::FourWeekColumns.weeks(), 4);
        assert_eq!(Layout::TwoWeekRows.weeks(), 2);
        assert_eq!(Layout::ThreeMonths.weeks(), 0);
    }

    #[test]
    fn six_months_crosses_the_year_boundary_without_skipping() {
        let got = months_from(Date::new(2026, 11, 10).unwrap(), 6);
        assert_eq!((got[0].year, got[0].month), (2026, 11));
        assert_eq!((got[2].year, got[2].month), (2027, 1));
        assert_eq!((got[5].year, got[5].month), (2027, 4));
    }

    #[test]
    fn weeks_counts_only_the_rows_the_month_actually_needs() {
        // の実体。並べている
        // 3か月(2026-09〜11)はどれも5週で収まるのに6行描いていた。
        let months = months_from(Date::new(2026, 9, 1).unwrap(), 3);
        assert_eq!(months.iter().map(Month::weeks).collect::<Vec<_>>(), [5, 5, 5]);
        // 土曜始まりの31日は6行、日曜始まりの28日は4行——上限も下限も外さない。
        assert_eq!(months_from(Date::new(2026, 8, 1).unwrap(), 1)[0].weeks(), 6);
        assert_eq!(months_from(Date::new(2026, 2, 1).unwrap(), 1)[0].weeks(), 4);
    }

    #[test]
    fn february_uses_the_real_number_of_days() {
        assert_eq!(months_from(Date::new(2028, 2, 1).unwrap(), 1)[0].days, 29);
        assert_eq!(months_from(Date::new(2027, 2, 1).unwrap(), 1)[0].days, 28);
    }

    #[test]
    fn month_titles_are_uppercase_three_letter_names() {
        assert_eq!(month_label(1), Some("JAN"));
        assert_eq!(month_label(8), Some("AUG"));
        assert_eq!(month_label(12), Some("DEC"));
        assert_eq!(month_label(13), None);
    }

    #[test]
    fn marks_round_trip_all_styles_colors_and_clear() {
        let colors = [
            MarkColor::Red,
            MarkColor::Gold,
            MarkColor::Blue,
            MarkColor::Green,
            MarkColor::Mauve,
        ];
        let mut marks = Marks::default();
        for (offset, color) in colors.into_iter().enumerate() {
            marks.set(
                Date::new(2026, 8, 17 + i8::try_from(offset).unwrap()).unwrap(),
                Some(Mark {
                    style: if offset % 2 == 0 {
                        MarkStyle::Text
                    } else {
                        MarkStyle::Underline
                    },
                    color,
                }),
            );
        }
        let path = std::env::temp_dir().join(format!(
            "cockpit-calendar-marks-{}.json",
            std::process::id()
        ));
        marks.save_to(&path).unwrap();
        let got = Marks::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(got, marks);
        let day = Date::new(2026, 8, 17).unwrap();
        marks.set(day, None);
        assert_eq!(marks.get(day), None);
    }

    #[test]
    fn old_weekend_colours_remain_readable_but_are_not_new_menu_choices() {
        let old = Marks::parse(
            r#"{"marks":{
                "2026-08-22":{"style":"underline","color":"blue"},
                "2026-08-23":{"style":"text","color":"red"}
            }}"#,
        )
        .unwrap();
        assert_eq!(
            old.get(Date::new(2026, 8, 22).unwrap()).unwrap().color,
            MarkColor::Blue
        );
        assert_eq!(
            old.get(Date::new(2026, 8, 23).unwrap()).unwrap().color,
            MarkColor::Red
        );
        assert_eq!(
            MarkColor::MENU,
            [MarkColor::Gold, MarkColor::Green, MarkColor::Mauve]
        );
        assert!(!MarkColor::MENU.contains(&MarkColor::Red));
        assert!(!MarkColor::MENU.contains(&MarkColor::Blue));
    }

    #[test]
    fn malformed_marks_never_become_guessed_values() {
        assert!(Marks::parse(r#"{"marks":{"2026-08-17":{"style":"dot","color":"red"}}}"#).is_err());
        assert!(
            Marks::parse(r#"{"marks":{"2026-08-17":{"style":"text","color":"purple"}}}"#).is_err()
        );
    }
}
