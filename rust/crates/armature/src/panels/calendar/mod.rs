//! The calendar: this week and the next few under the clock, today glowing; click it for six
//! months in the center, right-click a day to mark it. Japanese holidays are colored when the
//! Mac's region is Japan or the app speaks Japanese. How many weeks, and in rows or columns, is
//! `calendar.conf` in the state folder ([`data::layout`]); the marks are `calendar-marks.json`.

mod data;

use std::time::Duration;

use iced::widget::{Space, button, column, container, mouse_area, row, rule, scrollable, text};
use iced::{Alignment, Background, Border, Length, Subscription, Task};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::palette::{
    self, accent_alert, cal_holiday, cal_saturday, surface_active, surface_raised,
    text_faint, text_muted, text_primary, text_secondary,
};
use crate::{Element, Host, Notice, Panel, font, glow, tr};

use data as calendar;

/// Room between the small faces inside one panel (the three months).
const CARD_GAP: f32 = 6.0;

/// The calendar panel (`calendar` in `panels.conf`). Not in the starting arrangement.
pub struct Calendar {
    today: Date,
    layout: calendar::Layout,
    marks: calendar::Marks,
    /// The day whose mark menu is open (right-click).
    menu: Option<Date>,
    /// Why the marks could not be saved.
    error: Option<String>,
    /// Whether the six months are showing in the center.
    open: bool,
}

#[derive(Clone, Debug)]
pub enum Message {
    Tick,
    /// The panel was clicked: open or close the six months.
    Pressed,
    /// A day was right-clicked.
    Menu(Date),
    /// A mark was picked (or `None`: cleared).
    Mark(Option<calendar::Mark>),
}

impl Calendar {
    #[must_use]
    pub fn new() -> Self {
        calendar::prepare();
        Self {
            today: today(),
            layout: calendar::layout(),
            marks: calendar::Marks::load(),
            menu: None,
            error: None,
            open: false,
        }
    }
}

impl Default for Calendar {
    fn default() -> Self {
        Self::new()
    }
}

fn today() -> Date {
    Timestamp::now().to_zoned(TimeZone::system()).date()
}

impl Panel for Calendar {
    const KEY: &'static str = "calendar";
    const PADDED: bool = false;
    const SHOWN: bool = false;
    type Message = Message;

    fn view(&self) -> Element<'_, Message> {
        mouse_area(container(self.body()).padding(6).width(Length::Fill))
            .on_press(Message::Pressed)
            .into()
    }

    fn center(&self) -> Option<Element<'_, Message>> {
        self.open.then(|| {
            calendar_detail(self.today, &self.marks, self.menu, self.error.as_deref())
        })
    }

    fn update(&mut self, message: Message, host: &mut Host) -> Task<Message> {
        match message {
            Message::Tick => self.today = today(),
            Message::Pressed => {
                self.open = !self.open;
                self.menu = None;
                if !self.open {
                    host.release_keys();
                }
            }
            Message::Menu(date) => {
                self.menu = Some(date);
                self.error = None;
            }
            Message::Mark(mark) => {
                if let Some(date) = self.menu.take() {
                    self.marks.set(date, mark);
                    self.error = self.marks.save().err();
                    if self.error.is_some() {
                        self.menu = Some(date);
                    }
                }
            }
        }
        Task::none()
    }

    fn notice(&mut self, notice: &Notice, _host: &mut Host) -> Task<Message> {
        if matches!(notice, Notice::Dismiss) {
            self.open = false;
            self.menu = None;
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_secs(60)).map(|_| Message::Tick)
    }
}

impl Calendar {
    fn body(&self) -> Element<'_, Message> {
        match self.layout {
            calendar::Layout::ThreeMonths => self.months_body(),
            layout => self.weekly_body(layout),
        }
    }

    /// The weeks, and past them nothing: no per-day lines of events (the app has no events;
    /// holidays and marks are already on the grid).
    fn weekly_body(&self, layout: calendar::Layout) -> Element<'_, Message> {
        let weeks = calendar::weeks_from(self.today, layout.weeks());
        let inner: Element<'_, Message> = if layout == calendar::Layout::FourWeekColumns {
            weekly_columns_view(&weeks, self.today, &self.marks)
        } else {
            weekly_view(&weeks, self.today, &self.marks)
        };
        let grid = container(inner)
            .width(Length::Fill)
            .height(Length::Fixed(calendar_grid_height(layout)));
        let mut body = column![].spacing(8);
        if let Some(date) = self.menu.filter(|_| !self.open) {
            body = body.push(calendar_mark_menu(date, self.marks.get(date), self.error.as_deref()));
        }
        body.push(grid).into()
    }

    fn months_body(&self) -> Element<'_, Message> {
        let months = calendar::months_from(self.today, 3);
        // The three months are as tall as the one that needs the most weeks.
        let weeks = months
            .iter()
            .map(calendar::Month::weeks)
            .max()
            .unwrap_or(CALENDAR_WEEKS_MAX);
        let calendar_row = row![
            month_card(months[0].clone(), self.today, &self.marks, weeks),
            month_card(months[1].clone(), self.today, &self.marks, weeks),
            month_card(months[2].clone(), self.today, &self.marks, weeks),
        ]
        .spacing(CARD_GAP)
        .width(Length::Fill)
        .height(Length::Fixed(calendar_row_height(weeks)));
        let mut body = column![].spacing(8);
        if let Some(date) = self.menu.filter(|_| !self.open) {
            body = body.push(calendar_mark_menu(date, self.marks.get(date), self.error.as_deref()));
        }
        body.push(calendar_row).into()
    }
}

/// A small face floating over the panel (the mark menu), one step above it.
fn popover_style() -> container::Style {
    container::Style {
        text_color: Some(text_primary()),
        background: Some(Background::Color(surface_raised())),
        border: Border {
            radius: palette::radius_control().into(),
            ..Border::default()
        },
        ..Default::default()
    }
}

/// 週の暦の寸法。
/// 曜日の頭文字の段・1週の桝の丈・出す週数。
///
/// 2026-09-07 で一段詰めた
/// (曜日 12→11・桝 26→21・数字 13→11)。字を小さくしても桝の丈を据え置くと、
/// 数字の周りの空きだけが増えて逆に間延びする——**字と桝は一緒に縮める**。
const CALENDAR_WEEKDAY_HEIGHT: f32 = 11.0;
const CALENDAR_WEEKLY_ROW_HEIGHT: f32 = 21.0;
/// 週の暦の日付の字。時計と同じ Helvetica Neue(細)を [`font::CALENDAR_DIGITS`] で載せる。
const CALENDAR_WEEKLY_DAY_SIZE: f32 = 11.0;

/// 4週を縦の列で出すときの寸法。
/// 列の見出し(`9/6〜`)の段と、1日ぶんの行。
const CALENDAR_COLUMN_HEAD_HEIGHT: f32 = 12.0;
const CALENDAR_COLUMN_ROW_HEIGHT: f32 = 14.0;
/// 縦の列の日付は**一段小さい字**。7日を縦に積むので、横並びと同じ 13 では入らない。
const CALENDAR_COLUMN_DAY_SIZE: f32 = 10.0;

/// 格子の丈。見せ方ごとに段の数が違う。
fn calendar_grid_height(layout: calendar::Layout) -> f32 {
    match layout {
        // 列の見出し + 7日の行。
        calendar::Layout::FourWeekColumns => {
            CALENDAR_COLUMN_HEAD_HEIGHT + 2.0 + CALENDAR_COLUMN_ROW_HEIGHT * 7.0
        }
        // 曜日の段 + 週の桝(段の間 2px ずつ)。
        calendar::Layout::FourWeekRows | calendar::Layout::TwoWeekRows => {
            let weeks = layout.weeks() as f32;
            CALENDAR_WEEKDAY_HEIGHT + (CALENDAR_WEEKLY_ROW_HEIGHT + 2.0) * weeks
        }
        calendar::Layout::ThreeMonths => calendar_row_height(CALENDAR_WEEKS_MAX),
    }
}

/// 暦の1週ぶんの高さ。桝16 + 行間([`month_view`] の `grid` の spacing)2。
const CALENDAR_WEEK_HEIGHT: f32 = 18.0;
/// 1か月が要りうる週の行数の上限(31日 + 月初の空き6 = 37 → 6行)。
const CALENDAR_WEEKS_MAX: usize = 6;
/// 月カード1行ぶんの高さ。週行 18×週数(最後の行間は落ちるので -2)+
/// カード padding 上下12。
///
/// **週数は月によって違うので固定値にしない。**6週で決め打っていた頃は、
/// 並べた3枚とも5週で収まる月(2026-09〜11など)で最下段がまるごと空になり、
/// 暦の下に空白が1行開いて見えていた。
fn calendar_row_height(weeks: usize) -> f32 {
    weeks as f32 * CALENDAR_WEEK_HEIGHT - 2.0 + 12.0
}


/// 暦の月カードの地。**地は他のパネルと同じ段**([`surface_card`])。
fn month_card_style() -> container::Style {
    palette::card_style()
}

/// 暦に祝日の色を付けるか。日本の祝日は、Mac の地域が日本か表示が日本語のときだけ
/// (`armature_core::lang::japan_holidays`)。
fn is_marked_holiday(date: Date) -> bool {
    armature_core::lang::japan_holidays() && armature_core::holiday::is_holiday(date)
}



/// 週の暦の格子。曜日の頭文字を上に置き、今週と来週を1行ずつ並べる。
/// 日付の字は月のグリッド(10)より大きい。
///
/// 年をまたいで今月から6か月。日付は常設の週表示と同じ描画・印の操作を使う。
fn calendar_month_title(month: &calendar::Month) -> String {
    const EN: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    tr!(
        EN[usize::try_from(month.month.clamp(1, 12) - 1).unwrap_or(0)].to_string(),
        format!("{}月", month.month),
    )
}

fn calendar_detail<'a>(
    today: Date,
    marks: &'a calendar::Marks,
    menu: Option<Date>,
    error: Option<&'a str>,
) -> Element<'a, Message> {
    let months = calendar::months_from(today, 6);
    let mut body = column![].spacing(24);
    if let Some(date) = menu {
        body = body.push(calendar_mark_menu(date, marks.get(date), error));
    }
    for group in months.chunks(3) {
        let mut cards = row![].spacing(24);
        for month in group {
            let mut grid = column![
                text(calendar_month_title(month))
                    .size(18)
                    .color(text_primary())
            ]
            .spacing(12);
            let mut header = row![].spacing(1);
            for (weekday, label) in calendar::weekday_labels().iter().enumerate() {
                header = header.push(
                    container(
                        text(*label)
                            .size(12)
                            .color(calendar_day_color(false, false, false, weekday)),
                    )
                    .center_x(Length::FillPortion(1)),
                );
            }
            grid = grid.push(header);
            for week in 0..6 {
                let mut days = row![].spacing(1);
                for weekday in 0..7 {
                    let day = (week * 7 + weekday) as i8 - month.first_weekday as i8 + 1;
                    if day < 1 || day > month.days {
                        days = days.push(
                            Space::new()
                                .width(Length::FillPortion(1))
                                .height(Length::Fixed(CALENDAR_WEEKLY_ROW_HEIGHT)),
                        );
                    } else {
                        let date = Date::new(month.year, month.month, day).expect("calendar date");
                        days = days.push(weekly_cell(date, today, marks, weekday));
                    }
                }
                grid = grid.push(days);
            }
            cards = cards.push(
                container(grid)
                    .padding(12)
                    .width(Length::FillPortion(1))
                    .style(|_| palette::card_style()),
            );
        }
        body = body.push(cards);
    }
    container(scrollable(body))
        .padding(24)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}


fn weekly_view<'a>(
    weeks: &[[Date; 7]],
    today: Date,
    marks: &'a calendar::Marks,
) -> Element<'a, Message> {
    let mut header = row![].spacing(1);
    for (weekday, label) in calendar::weekday_labels().iter().enumerate() {
        // 曜日の頭文字は日付ではないので「今日」も「過去」も持たない。
        // 色は週末だけを分ける規則をそのまま借りる。
        let color = calendar_day_color(false, false, false, weekday);
        header = header.push(
            container(text(*label).size(7).color(color))
                .center_x(Length::FillPortion(1))
                .height(Length::Fixed(CALENDAR_WEEKDAY_HEIGHT)),
        );
    }
    let mut grid = column![header].spacing(2);
    for week in weeks {
        let mut cells = row![].spacing(1);
        for (weekday, date) in week.iter().enumerate() {
            cells = cells.push(weekly_cell(*date, today, marks, weekday));
        }
        grid = grid.push(cells);
    }
    container(grid).width(Length::Fill).into()
}

/// 週の暦の1桝。
fn weekly_cell<'a>(
    date: Date,
    today: Date,
    marks: &'a calendar::Marks,
    weekday: usize,
) -> Element<'a, Message> {
    let is_today = date == today;
    let is_past = date < today;
    let color = calendar_day_color(
        is_today,
        is_past,
        is_marked_holiday(date),
        weekday,
    );
    // 過ぎた日は印を出さない。月のグリッドと同じ規則。
    let mark = if is_past { None } else { marks.get(date) };
    let text_color = mark
        .filter(|mark| mark.style == calendar::MarkStyle::Text)
        .map(|mark| calendar_mark_color(mark.color))
        .unwrap_or(color);
    // **どの日も数字だけ**。月をまたぐ日に「10/1」と添えていたが、4行を上から順に読めば
    // どこで月が変わったかは 31 の次が 1 になることで分かる——添え字は余計だった。
    let number = container(calendar_digits(
        date.day().to_string(),
        CALENDAR_WEEKLY_DAY_SIZE,
        text_color,
        is_today,
    ))
    .center_x(Length::Fill)
    .height(Length::Fixed(14.0));
    let content: Element<'a, Message> =
        if let Some(mark) = mark.filter(|mark| mark.style == calendar::MarkStyle::Underline) {
            let underline = calendar_mark_color(mark.color);
            column![
                number,
                rule::horizontal(1).style(move |_| rule::Style {
                    color: underline,
                    radius: 1.0.into(),
                    fill_mode: rule::FillMode::Percent(58.0),
                    snap: true,
                }),
            ]
            .spacing(0)
            .height(Length::Fixed(CALENDAR_WEEKLY_ROW_HEIGHT))
            .into()
        } else {
            container(number)
                .center_y(Length::Fixed(CALENDAR_WEEKLY_ROW_HEIGHT))
                .into()
        };
    let cell = container(content)
        .center_x(Length::Fill)
        .center_y(Length::Fixed(CALENDAR_WEEKLY_ROW_HEIGHT));
    mouse_area(cell.width(Length::FillPortion(1)))
        .on_right_press(Message::Menu(date))
        .into()
}

/// 暦の日付の数字。**今日だけは細字をやめて光らせる**——桝を面で囲うと、線で仕切った
/// 画面の中にそこだけ箱が浮く。時計と同じ光り方で「いま」を指す。
fn calendar_digits<'a>(label: String, size: f32, color: iced::Color, is_today: bool) -> Element<'a, Message> {
    if !is_today {
        return text(label)
            .font(font::CALENDAR_DIGITS)
            .size(size)
            .color(color)
            .into();
    }
    glow::glow(
        move |ink| {
            text(label.clone())
                .font(font::CALENDAR_TODAY)
                .size(size)
                .color(ink)
                .into()
        },
        color,
        CALENDAR_TODAY_GLOW,
    )
}

/// 暦の今日の滲み。字が時計の5分の1ほどなので、同じ濃さだと滲みが字を食う。
const CALENDAR_TODAY_GLOW: f32 = 0.6;

/// 4週を**縦の列**で出す。
/// 左から今週・来週・再来週・4週目。列の中は日〜土が縦に並ぶ。
///
/// 曜日の頭文字は列の外の1段ではなく**各行の頭に付く**——列が縦なので、
/// 上に1段置いても行と対応しない。
fn weekly_columns_view<'a>(
    weeks: &[[Date; 7]],
    today: Date,
    marks: &'a calendar::Marks,
) -> Element<'a, Message> {
    let mut columns = row![].spacing(CARD_GAP);
    for week in weeks {
        // 見出しは週の起点。「今週/来週」ではなく日付にしてある——4列も並ぶと
        // 相対の言葉のほうが数えにくい。
        let head = text(format!("{}/{}〜", week[0].month(), week[0].day()))
            .size(8)
            .color(if week.contains(&today) {
                text_secondary()
            } else {
                text_faint()
            });
        let mut list = column![
            container(head)
                .center_x(Length::Fill)
                .height(Length::Fixed(CALENDAR_COLUMN_HEAD_HEIGHT))
        ]
        .spacing(0);
        for (weekday, date) in week.iter().enumerate() {
            list = list.push(weekly_column_cell(*date, today, marks, weekday));
        }
        columns = columns.push(container(list).width(Length::FillPortion(1)));
    }
    container(columns).width(Length::Fill).into()
}

/// 縦の列の1日。曜日の頭文字 + 日付を1行に。
fn weekly_column_cell<'a>(
    date: Date,
    today: Date,
    marks: &'a calendar::Marks,
    weekday: usize,
) -> Element<'a, Message> {
    let is_today = date == today;
    let is_past = date < today;
    let color = calendar_day_color(
        is_today,
        is_past,
        is_marked_holiday(date),
        weekday,
    );
    let mark = if is_past { None } else { marks.get(date) };
    let day_color = mark
        .filter(|mark| mark.style == calendar::MarkStyle::Text)
        .map(|mark| calendar_mark_color(mark.color))
        .unwrap_or(color);
    // どの日も数字だけ(横並びと同じ規則。上の `weekly_cell` の頭注)。
    let label = date.day().to_string();
    let line = row![
        text(calendar::weekday_labels()[weekday])
            .size(7)
            .color(palette::with_alpha(color, 0.75))
            .width(Length::Fixed(11.0)),
        calendar_digits(label, CALENDAR_COLUMN_DAY_SIZE, day_color, is_today),
    ]
    .spacing(2)
    .align_y(Alignment::Center);
    let content: Element<'a, Message> =
        if let Some(mark) = mark.filter(|mark| mark.style == calendar::MarkStyle::Underline) {
            let underline = calendar_mark_color(mark.color);
            column![
                container(line).height(Length::Fixed(CALENDAR_COLUMN_ROW_HEIGHT - 2.0)),
                rule::horizontal(1).style(move |_| rule::Style {
                    color: underline,
                    radius: 1.0.into(),
                    fill_mode: rule::FillMode::Percent(70.0),
                    snap: true,
                }),
            ]
            .spacing(0)
            .into()
        } else {
            line.into()
        };
    let cell = container(content)
        .center_x(Length::Fill)
        .center_y(Length::Fixed(CALENDAR_COLUMN_ROW_HEIGHT));
    mouse_area(cell.width(Length::Fill))
        .on_right_press(Message::Menu(date))
        .into()
}

fn month_card<'a>(
    month: calendar::Month,
    today: Date,
    marks: &'a calendar::Marks,
    weeks: usize,
) -> Element<'a, Message> {
    container(month_view(month, today, marks, weeks))
        .padding([6, 3])
        .width(Length::FillPortion(1))
        .height(Length::Fill)
        .style(move |_| month_card_style())
        .into()
}

fn month_view<'a>(
    month: calendar::Month,
    today: Date,
    marks: &'a calendar::Marks,
    weeks: usize,
) -> Element<'a, Message> {
    // 月名(AUG / SEP / OCT)は出さない。今日の印がある1枚目を起点に、右へ
    // 翌月・翌々月と読めば足りる——3行ぶんの縦をニュース帯へ回した。
    let mut grid = column![].spacing(2);

    let mut day = 1_i8;
    for week in 0..weeks.clamp(month.weeks(), CALENDAR_WEEKS_MAX) {
        let mut cells = row![].spacing(1);
        for weekday in 0..7 {
            let index = week * 7 + weekday;
            if index < month.first_weekday || day > month.days {
                cells = cells.push(Space::new().width(Length::FillPortion(1)));
                continue;
            }
            let is_today =
                month.year == today.year() && month.month == today.month() && day == today.day();
            let is_past =
                month.year == today.year() && month.month == today.month() && day < today.day();
            let date = Date::new(month.year, month.month, day).expect("calendar dates are valid");
            let color = calendar_day_color(
                is_today,
                is_past,
                is_marked_holiday(date),
                weekday,
            );
            // 過ぎた日は印を出さない。字の色も下線も付けず、
            // 過去色のまま沈める——印の付け外し(右押し)と保存した値はそのまま。
            let mark = if is_past { None } else { marks.get(date) };
            let text_color = mark
                .filter(|mark| mark.style == calendar::MarkStyle::Text)
                .map(|mark| calendar_mark_color(mark.color))
                .unwrap_or(color);
            let number = container(calendar_digits(day.to_string(), 10.0, text_color, is_today))
                .center_x(Length::Fill)
                .height(Length::Fixed(13.0));
            let content: Element<'a, Message> = if let Some(mark) =
                mark.filter(|mark| mark.style == calendar::MarkStyle::Underline)
            {
                let underline = calendar_mark_color(mark.color);
                column![
                    number,
                    rule::horizontal(1).style(move |_| rule::Style {
                        color: underline,
                        radius: 1.0.into(),
                        fill_mode: rule::FillMode::Percent(58.0),
                        snap: true,
                    }),
                ]
                .spacing(0)
                .height(Length::Fixed(16.0))
                .into()
            } else {
                container(number).center_y(Length::Fixed(16.0)).into()
            };
            let cell = container(content)
                .center_x(Length::Fill)
                .center_y(Length::Fixed(16.0));
            cells = cells.push(
                mouse_area(cell.width(Length::FillPortion(1)))
                    .on_right_press(Message::Menu(date)),
            );
            day += 1;
        }
        grid = grid.push(cells);
    }

    container(grid).width(Length::Fill).into()
}

fn calendar_mark_menu(
    date: Date,
    selected: Option<calendar::Mark>,
    error: Option<&str>,
) -> Element<'static, Message> {
    let mut text_marks = row![text("Text").size(9).color(text_muted())]
        .spacing(4)
        .align_y(Alignment::Center);
    let mut underlines = row![text("Underline").size(9).color(text_muted())]
        .spacing(4)
        .align_y(Alignment::Center);
    for color in calendar::MarkColor::MENU {
        let text_mark = calendar::Mark {
            style: calendar::MarkStyle::Text,
            color,
        };
        let underline_mark = calendar::Mark {
            style: calendar::MarkStyle::Underline,
            color,
        };
        text_marks = text_marks.push(calendar_mark_button("A", text_mark, selected));
        underlines = underlines.push(calendar_mark_button("_", underline_mark, selected));
    }
    let clear = button(text("Clear").size(9).color(text_faint()))
        .padding([2, 5])
        .on_press(Message::Mark(None));
    let mut body = column![
        row![
            text(date.to_string()).size(10).color(text_primary()),
            Space::new().width(Length::Fill),
            clear,
        ]
        .align_y(Alignment::Center),
        text_marks,
        underlines,
    ]
    .spacing(3);
    if let Some(error) = error {
        body = body.push(text(error.to_string()).size(9).color(accent_alert()));
    }
    container(body)
        .padding([5, 7])
        .width(Length::Shrink)
        .style(|_| popover_style())
        .into()
}

fn calendar_mark_button(
    label: &'static str,
    mark: calendar::Mark,
    selected: Option<calendar::Mark>,
) -> iced::widget::Button<'static, Message> {
    let color = calendar_mark_color(mark.color);
    button(text(label).size(12).color(color))
        .padding([1, 7])
        .on_press(Message::Mark(Some(mark)))
        .style(move |_, status| button::Style {
            background: if selected == Some(mark) {
                Some(Background::Color(surface_active()))
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                Some(Background::Color(surface_raised()))
            } else {
                None
            },
            text_color: color,
            border: Border {
                width: 0.0,
                radius: palette::radius_control().into(),
                ..Border::default()
            },
            ..button::Style::default()
        })
}

fn calendar_mark_color(color: calendar::MarkColor) -> iced::Color {
    match color {
        calendar::MarkColor::Red => palette::flamingo(),
        calendar::MarkColor::Gold => palette::yellow(),
        calendar::MarkColor::Blue => palette::blue(),
        calendar::MarkColor::Green => palette::green(),
        // 名前どおりの藤色へ。`MOCHA_SONNET`(ピンク)の流用をやめた——
        // モデル表示では引き続き使うので、色そのものは消していない。
        calendar::MarkColor::Mauve => palette::mauve(),
    }
}

/// 過ぎた日の薄さ。**いちばん薄い本文色([`text_faint`])をさらに落とす。**
///
/// 素の `text_faint` でも「まだ読める字」として先の日付と競っていた。
/// 色そのものは変えず透かすので、色域を切り替えても薄さの関係は崩れない。
const CALENDAR_PAST_ALPHA: f32 = 0.45;

fn calendar_day_color(
    is_today: bool,
    is_past: bool,
    is_holiday: bool,
    weekday: usize,
) -> iced::Color {
    if is_today {
        palette::text_lit()
    } else if is_past {
        palette::with_alpha(text_faint(), CALENDAR_PAST_ALPHA)
    } else if is_holiday || weekday == 0 {
        cal_holiday()
    } else if weekday == 6 {
        cal_saturday()
    } else {
        text_muted()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_titles_omit_the_year_even_across_new_year() {
        let months = calendar::months_from(Date::new(2026, 11, 1).unwrap(), 6);
        let ja = armature_core::lang::scoped(armature_core::lang::Lang::Ja);
        let titles: Vec<_> = months.iter().map(calendar_month_title).collect();
        assert_eq!(titles, ["11月", "12月", "1月", "2月", "3月", "4月"]);
        drop(ja);
        let _en = armature_core::lang::scoped(armature_core::lang::Lang::En);
        let titles: Vec<_> = months.iter().map(calendar_month_title).collect();
        assert_eq!(titles, ["Nov", "Dec", "Jan", "Feb", "Mar", "Apr"]);
        assert_eq!(months[2].year, 2027);
    }

    #[test]
    fn the_calendar_row_shrinks_by_a_whole_week_when_no_month_needs_six() {
        // 6週ぶん = 従来の決め打ちと同じ高さ(1px の取りこぼしだけ直っている)。
        assert_eq!(calendar_row_height(CALENDAR_WEEKS_MAX), 118.0);
        // 5週で収まるなら1行ぶん(18px)まるごと縮む。ここが空白の正体だった。
        assert_eq!(
            calendar_row_height(CALENDAR_WEEKS_MAX) - calendar_row_height(5),
            CALENDAR_WEEK_HEIGHT
        );
        assert_eq!(calendar_row_height(5), 100.0);
    }

    #[test]
    fn calendar_mark_palette_has_three_distinct_choices() {
        let colors = calendar::MarkColor::MENU.map(calendar_mark_color);
        assert_ne!(colors[0], colors[1]);
        assert_ne!(colors[1], colors[2]);
        assert_ne!(colors[0], colors[2]);
        for color in colors {
            assert_ne!(color, calendar_day_color(false, false, false, 0));
            assert_ne!(color, calendar_day_color(false, false, false, 6));
        }
    }

    /// 新しく付けられる印の色は、暦の日付の色のどれとも取り違えられない。
    ///
    /// 藤色が `MOCHA_SONNET`(ピンク)だった頃、日曜・祝日の `CAL_HOLIDAY` と
    /// ΔE00 12.2 しか離れておらず利用者が「赤色と似すぎてて使えない」と裁定した
    /// (2026-08-18)。本来の Frappé mauve へ戻して 22.0 まで離してある。
    #[test]
    fn every_pickable_mark_color_is_distinct_from_the_day_colors() {
        let day_colors = [
            ("日曜・祝日", cal_holiday()),
            ("土曜", cal_saturday()),
            ("今日", text_primary()),
            ("過去", text_faint()),
        ];
        for mark in calendar::MarkColor::MENU {
            let color = calendar_mark_color(mark);
            for (name, day) in day_colors {
                assert_ne!(color, day, "{mark:?} が {name} と同じ色");
            }
        }
        // 藤色は名前どおりの藤色で、ピンクの流用ではない。
        assert_eq!(
            calendar_mark_color(calendar::MarkColor::Mauve),
            palette::mauve()
        );
        assert_ne!(palette::mauve(), iced::Color::from_rgb8(0xf5, 0xc2, 0xe7));
    }

    #[test]
    fn sundays_and_saturdays_keep_distinct_calendar_colors() {
        let holiday = Date::new(2026, 9, 21).unwrap();
        assert!(armature_core::holiday::is_holiday(holiday));
        assert_eq!(calendar_day_color(false, false, true, 1), cal_holiday());
        assert_eq!(calendar_day_color(false, false, false, 0), cal_holiday());
        assert_eq!(calendar_day_color(false, false, false, 6), cal_saturday());
        assert_ne!(
            calendar_day_color(false, false, false, 0),
            calendar_day_color(false, false, false, 6)
        );
        // 過ぎた日は曜日も祝日も関係なく1色。**いちばん薄い本文色より、
        // さらに薄い**。
        let past = palette::with_alpha(text_faint(), CALENDAR_PAST_ALPHA);
        assert_eq!(calendar_day_color(false, true, false, 2), past);
        assert_eq!(calendar_day_color(false, true, true, 2), past);
        assert_eq!(calendar_day_color(false, true, false, 0), past);
        assert_eq!(calendar_day_color(false, true, false, 6), past);
        assert!(past.a < text_faint().a, "素の薄さのままになっている");
        // 今日は過ぎた日の扱いに落ちず、光る字の芯の色になる。
        assert_eq!(calendar_day_color(true, true, true, 0), palette::text_lit());
    }

    /// 見せ方を切り替えてもパネルの外形が跳ねないこと。
    #[test]
    fn 暦の格子は見せ方ごとに段の数だけ違う() {
        // 4週の列: 見出し 12 + 隙間 2 + 7日 × 14 = 112。
        assert!((calendar_grid_height(calendar::Layout::FourWeekColumns) - 112.0).abs() < 0.01);
        // 4週の行: 曜日 11 + 4週 × 23 = 103(2026-09-07 に一段詰めた)。
        assert!((calendar_grid_height(calendar::Layout::FourWeekRows) - 103.0).abs() < 0.01);
        // 2週の行はその半分ぶん短い。
        assert!((calendar_grid_height(calendar::Layout::TwoWeekRows) - 57.0).abs() < 0.01);
        // 縦と横で 12px しか違わない——切り替えてもパネルが跳ねない。
        let gap = (calendar_grid_height(calendar::Layout::FourWeekColumns)
            - calendar_grid_height(calendar::Layout::FourWeekRows))
        .abs();
        assert!(gap <= 16.0, "縦と横の丈の差が大きすぎる: {gap}");
        // 3か月は従来どおり。
        assert!(
            (calendar_grid_height(calendar::Layout::ThreeMonths)
                - calendar_row_height(CALENDAR_WEEKS_MAX))
            .abs()
                < 0.01
        );
        // 縦の列の字は横並びより一段小さい(7日を縦に積むため)。
        const { assert!(CALENDAR_COLUMN_DAY_SIZE < CALENDAR_WEEKLY_DAY_SIZE) };
        // 数字は桝に収まる。
        // 行送り 1.3 ぶんを見込んでも桝の丈を超えない。
        const { assert!(CALENDAR_WEEKLY_DAY_SIZE * 1.3 < CALENDAR_WEEKLY_ROW_HEIGHT) };
    }
}
