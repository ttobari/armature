//! 窓の絵の具。配色(有名な配色の数組)と、役割から色への写像を持つ。
//!
//! 構成は3層。
//!
//! - §1 配色 —— [`Theme`] の1組ぶんの17色と画面の色。**導出元**であって、パネルから直に呼ばない
//! - §2 役割 —— パネルが使うのはこちら。面の高度・文字の階層・意味色・寸法
//! - §3 model / effort —— Claude Code のモデルと思考量の色
//!
//! 規律は3つ。
//!
//! 1. 窓は1枚の画面。パネルどうしは面の明度差ではなく 1px の線([`surface_line`])で
//!    区切る。面を浮かせるのは束の見出し・ホバー・入力欄のような小さいものだけ
//! 2. アクセント色は状態や意味を示すときだけ。見出しや章の色分けのような
//!    装飾では出さない。装飾で階層を出したいときは文字の階層([`text_secondary`]
//!    以下)を使う
//! 3. 無彩色グレー(R=G=B)を新設しない。配色の中立色から導出する

use std::sync::atomic::{AtomicU8, Ordering};

use iced::widget::container;
use iced::{Background, Border, Color};

// ── §1 配色 ─────────────────────────────────────────────────────────────
//
// 17色の名は Catppuccin の役割名を借りる(ほかの配色も同じ役割へ写す)。
// 中立色は暗い順に screen <= crust < base < surface0 < surface1。

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

/// 選べる配色。**暗い配色だけ**——光る字(時計・今日・選んでいる行)は暗い地の上で
/// 白へ寄せて光らせる作りなので、明るい地では芯が地に溶ける。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    CatppuccinFrappe,
    CatppuccinMocha,
    TokyoNight,
    Nord,
    Gruvbox,
    Dracula,
}

impl Theme {
    pub const ALL: [Self; 6] = [
        Self::CatppuccinFrappe,
        Self::CatppuccinMocha,
        Self::TokyoNight,
        Self::Nord,
        Self::Gruvbox,
        Self::Dracula,
    ];

    /// 設定の画面に出す名。配色の固有名なので訳さない。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::CatppuccinFrappe => "Catppuccin Frappé",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::TokyoNight => "Tokyo Night",
            Self::Nord => "Nord",
            Self::Gruvbox => "Gruvbox",
            Self::Dracula => "Dracula",
        }
    }

    /// 保存に使う綴り(状態の置き場の `theme`)。
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::CatppuccinFrappe => "catppuccin-frappe",
            Self::CatppuccinMocha => "catppuccin-mocha",
            Self::TokyoNight => "tokyo-night",
            Self::Nord => "nord",
            Self::Gruvbox => "gruvbox",
            Self::Dracula => "dracula",
        }
    }

    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.key() == key.trim())
    }

    const fn ink(self) -> &'static Ink {
        match self {
            Self::CatppuccinFrappe => &FRAPPE,
            Self::CatppuccinMocha => &MOCHA,
            Self::TokyoNight => &TOKYO_NIGHT,
            Self::Nord => &NORD,
            Self::Gruvbox => &GRUVBOX,
            Self::Dracula => &DRACULA,
        }
    }
}

/// 配色1組。値は各配色の公式の色(画面の色だけはこちらで選ぶ)。
struct Ink {
    /// 窓の地とパネルの面。配色のいちばん暗い地か、それより一段沈めた色。
    screen: Color,
    crust: Color,
    base: Color,
    surface0: Color,
    surface1: Color,
    text: Color,
    subtext1: Color,
    subtext0: Color,
    overlay1: Color,
    green: Color,
    yellow: Color,
    red: Color,
    blue: Color,
    maroon: Color,
    peach: Color,
    sky: Color,
    flamingo: Color,
    mauve: Color,
}

/// Catppuccin Frappé。画面は crust を 35% 沈めた黒(青みを残したまま字と線が立つ深さ)。
const FRAPPE: Ink = Ink {
    screen: rgb(0x18, 0x19, 0x22),
    crust: rgb(0x23, 0x26, 0x34),
    base: rgb(0x30, 0x34, 0x46),
    surface0: rgb(0x41, 0x45, 0x59),
    surface1: rgb(0x51, 0x57, 0x6d),
    text: rgb(0xc6, 0xd0, 0xf5),
    subtext1: rgb(0xb5, 0xbf, 0xe2),
    subtext0: rgb(0xa5, 0xad, 0xce),
    overlay1: rgb(0x83, 0x8b, 0xa7),
    green: rgb(0xa6, 0xd1, 0x89),
    yellow: rgb(0xe5, 0xc8, 0x90),
    red: rgb(0xe7, 0x82, 0x84),
    blue: rgb(0x8c, 0xaa, 0xee),
    maroon: rgb(0xea, 0x99, 0x9c),
    peach: rgb(0xef, 0x9f, 0x76),
    sky: rgb(0x99, 0xd1, 0xdb),
    flamingo: rgb(0xee, 0xbe, 0xbe),
    mauve: rgb(0xca, 0x9e, 0xe6),
};

/// Catppuccin Mocha。
const MOCHA: Ink = Ink {
    screen: rgb(0x11, 0x11, 0x1b),
    crust: rgb(0x11, 0x11, 0x1b),
    base: rgb(0x1e, 0x1e, 0x2e),
    surface0: rgb(0x31, 0x32, 0x44),
    surface1: rgb(0x45, 0x47, 0x5a),
    text: rgb(0xcd, 0xd6, 0xf4),
    subtext1: rgb(0xba, 0xc2, 0xde),
    subtext0: rgb(0xa6, 0xad, 0xc8),
    overlay1: rgb(0x7f, 0x84, 0x9c),
    green: rgb(0xa6, 0xe3, 0xa1),
    yellow: rgb(0xf9, 0xe2, 0xaf),
    red: rgb(0xf3, 0x8b, 0xa8),
    blue: rgb(0x89, 0xb4, 0xfa),
    maroon: rgb(0xeb, 0xa0, 0xac),
    peach: rgb(0xfa, 0xb3, 0x87),
    sky: rgb(0x89, 0xdc, 0xeb),
    flamingo: rgb(0xf2, 0xcd, 0xcd),
    mauve: rgb(0xcb, 0xa6, 0xf7),
};

/// Tokyo Night(night)。
const TOKYO_NIGHT: Ink = Ink {
    screen: rgb(0x16, 0x16, 0x1e),
    crust: rgb(0x16, 0x16, 0x1e),
    base: rgb(0x1a, 0x1b, 0x26),
    surface0: rgb(0x29, 0x2e, 0x42),
    surface1: rgb(0x41, 0x48, 0x68),
    text: rgb(0xc0, 0xca, 0xf5),
    subtext1: rgb(0xa9, 0xb1, 0xd6),
    subtext0: rgb(0x9a, 0xa5, 0xce),
    overlay1: rgb(0x73, 0x7a, 0xa2),
    green: rgb(0x9e, 0xce, 0x6a),
    yellow: rgb(0xe0, 0xaf, 0x68),
    red: rgb(0xf7, 0x76, 0x8e),
    blue: rgb(0x7a, 0xa2, 0xf7),
    maroon: rgb(0xff, 0x89, 0x9d),
    peach: rgb(0xff, 0x9e, 0x64),
    sky: rgb(0x7d, 0xcf, 0xff),
    flamingo: rgb(0xf7, 0x76, 0x8e),
    mauve: rgb(0xbb, 0x9a, 0xf7),
};

/// Nord。画面は Polar Night(nord0)より一段沈めた色——nord0 のままだと線が立たない。
const NORD: Ink = Ink {
    screen: rgb(0x24, 0x29, 0x33),
    crust: rgb(0x2e, 0x34, 0x40),
    base: rgb(0x2e, 0x34, 0x40),
    surface0: rgb(0x3b, 0x42, 0x52),
    surface1: rgb(0x43, 0x4c, 0x5e),
    text: rgb(0xd8, 0xde, 0xe9),
    subtext1: rgb(0xc2, 0xc9, 0xd6),
    subtext0: rgb(0xa3, 0xab, 0xb9),
    overlay1: rgb(0x7b, 0x88, 0xa1),
    green: rgb(0xa3, 0xbe, 0x8c),
    yellow: rgb(0xeb, 0xcb, 0x8b),
    red: rgb(0xbf, 0x61, 0x6a),
    blue: rgb(0x81, 0xa1, 0xc1),
    maroon: rgb(0xc5, 0x72, 0x7a),
    peach: rgb(0xd0, 0x87, 0x70),
    sky: rgb(0x88, 0xc0, 0xd0),
    flamingo: rgb(0xbf, 0x61, 0x6a),
    mauve: rgb(0xb4, 0x8e, 0xad),
};

/// Gruvbox(dark・hard)。
const GRUVBOX: Ink = Ink {
    screen: rgb(0x1d, 0x20, 0x21),
    crust: rgb(0x1d, 0x20, 0x21),
    base: rgb(0x28, 0x28, 0x28),
    surface0: rgb(0x3c, 0x38, 0x36),
    surface1: rgb(0x50, 0x49, 0x45),
    text: rgb(0xeb, 0xdb, 0xb2),
    subtext1: rgb(0xd5, 0xc4, 0xa1),
    subtext0: rgb(0xbd, 0xae, 0x93),
    overlay1: rgb(0x92, 0x83, 0x74),
    green: rgb(0xb8, 0xbb, 0x26),
    yellow: rgb(0xfa, 0xbd, 0x2f),
    red: rgb(0xfb, 0x49, 0x34),
    blue: rgb(0x83, 0xa5, 0x98),
    maroon: rgb(0xfb, 0x49, 0x34),
    peach: rgb(0xfe, 0x80, 0x19),
    sky: rgb(0x8e, 0xc0, 0x7c),
    flamingo: rgb(0xfb, 0x49, 0x34),
    mauve: rgb(0xd3, 0x86, 0x9b),
};

/// Dracula。
const DRACULA: Ink = Ink {
    screen: rgb(0x21, 0x22, 0x2c),
    crust: rgb(0x19, 0x1a, 0x21),
    base: rgb(0x28, 0x2a, 0x36),
    surface0: rgb(0x34, 0x37, 0x46),
    surface1: rgb(0x44, 0x47, 0x5a),
    text: rgb(0xf8, 0xf8, 0xf2),
    subtext1: rgb(0xe2, 0xe2, 0xdc),
    subtext0: rgb(0xb6, 0xb8, 0xc6),
    overlay1: rgb(0x62, 0x72, 0xa4),
    green: rgb(0x50, 0xfa, 0x7b),
    yellow: rgb(0xf1, 0xfa, 0x8c),
    red: rgb(0xff, 0x55, 0x55),
    blue: rgb(0xbd, 0x93, 0xf9),
    maroon: rgb(0xff, 0x6e, 0x6e),
    peach: rgb(0xff, 0xb8, 0x6c),
    sky: rgb(0x8b, 0xe9, 0xfd),
    flamingo: rgb(0xff, 0x79, 0xc6),
    mauve: rgb(0xbd, 0x93, 0xf9),
};

/// いまの配色([`Theme::ALL`] の添字)。描くたびに何百回と読むので原子の1バイト。
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// いまの配色。
#[must_use]
pub fn theme() -> Theme {
    #[cfg(test)]
    if let Some(theme) = TEST_THEME.with(std::cell::Cell::get) {
        return theme;
    }
    Theme::ALL
        .get(usize::from(CURRENT.load(Ordering::Relaxed)))
        .copied()
        .unwrap_or(Theme::CatppuccinFrappe)
}

/// 配色を差し替える。次の描画から窓ぜんぶが入れ替わる(端末の色は呼ぶ側が送り直す)。
pub fn set_theme(theme: Theme) {
    let index = Theme::ALL.iter().position(|one| *one == theme).unwrap_or(0);
    #[allow(clippy::cast_possible_truncation)]
    CURRENT.store(index as u8, Ordering::Relaxed);
}

// 試験は配色をスレッドの中でだけ差し替える(並んで走るほかの試験の色を動かさない)。
#[cfg(test)]
thread_local! {
    static TEST_THEME: std::cell::Cell<Option<Theme>> = const { std::cell::Cell::new(None) };
}

/// 試験用。このスレッドの中でだけ配色を差し替える。
#[cfg(test)]
pub fn set_test_theme(theme: Option<Theme>) {
    TEST_THEME.with(|cell| cell.set(theme));
}

fn ink() -> &'static Ink {
    theme().ink()
}

macro_rules! ink {
    ($($name:ident),+ $(,)?) => {
        $(
            // 17色は全部口を開けておく(パネルを足すときに使う)。
            #[allow(dead_code)]
            #[must_use]
            pub fn $name() -> Color {
                ink().$name
            }
        )+
    };
}

ink!(
    crust, base, surface0, surface1, subtext1, subtext0, overlay1, green, yellow, red, blue,
    maroon, peach, sky, flamingo, mauve,
);

/// 本文の絵の具。`iced::widget::text` と名が当たるので `_ink` を付ける。
#[must_use]
pub fn text_ink() -> Color {
    ink().text
}

// ── 色の算術 ────────────────────────────────────────────────────────────

/// 2色を混ぜる。`amount` が 0.0 で `from`、1.0 で `to`。
#[must_use]
pub fn mix(from: Color, to: Color, amount: f32) -> Color {
    let t = amount.clamp(0.0, 1.0);
    Color {
        r: from.r + (to.r - from.r) * t,
        g: from.g + (to.g - from.g) * t,
        b: from.b + (to.b - from.b) * t,
        a: from.a + (to.a - from.a) * t,
    }
}

/// 白へ寄せる。
#[must_use]
pub fn lighten(color: Color, amount: f32) -> Color {
    mix(
        color,
        Color {
            a: color.a,
            ..Color::WHITE
        },
        amount,
    )
}

/// 透かす。
#[must_use]
pub fn with_alpha(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

// ── §2-a 画面と線、面の高度差 ─────────────────────────────────────────────
//
// 窓の地とパネルは同じ1枚の画面(SCREEN)。その上で浮くものが RAISED < ACTIVE。

/// 窓の地。パネルと同じ画面の色で、隙間は作らない(境は [`surface_line`] が引く)。
#[must_use]
pub fn surface_window() -> Color {
    ink().screen
}

/// パネルの面。地と同じ画面の色——パネルはカードとして浮かせず、線で仕切る。
#[must_use]
pub fn surface_card() -> Color {
    ink().screen
}

/// パネルの境の線(1px)。画面を surface1 へ3分の1だけ寄せた、同じ色相の線
/// (Frappé で #2b2e3b)。これより淡いと 1px では見えず、濃いと席の数だけ格子が浮く。
#[must_use]
pub fn surface_line() -> Color {
    mix(ink().screen, ink().surface1, LINE_LIFT)
}

const LINE_LIFT: f32 = 0.33;

/// カードの上でさらに浮くもの。行のホバー・入力欄・チップ。
#[must_use]
pub fn surface_raised() -> Color {
    surface0()
}

/// 選択中・アクティブな行。[`surface_raised`] のもう1段上。
#[must_use]
pub fn surface_active() -> Color {
    surface1()
}

/// 入力欄の縁のような、部品そのものの輪郭。
#[must_use]
pub fn surface_edge() -> Color {
    surface_line()
}

// ── §2-b 文字の階層 ──────────────────────────────────────────────────────
//
// PRIMARY > SECONDARY > MUTED > FAINT。読ませたい順にこの順で落とす。

/// 本文・主な値。読ませたいものはこれ。
#[must_use]
pub fn text_primary() -> Color {
    text_ink()
}

/// パネルの見出し・キー名・章の名。本文より一段引くが、まだ読ませる。
#[must_use]
pub fn text_secondary() -> Color {
    subtext1()
}

/// 補助情報。日付・件数・パス・単位。
#[must_use]
pub fn text_muted() -> Color {
    subtext0()
}

/// 非活性・既読・目盛り。あることだけ分かればよいもの。
#[must_use]
pub fn text_faint() -> Color {
    overlay1()
}

/// 光る字の芯。「いま」を指すもの——時計・暦の今日・選んでいる行——だけに使う。
/// 本文を白へ半分寄せた色で、まわりの滲みは [`crate::glow`] が足す。
#[must_use]
pub fn text_lit() -> Color {
    lighten(text_ink(), 0.5)
}

/// 沈みの深さ(1.0 = 沈めない)。
///
/// **役割色の1段落とし(`text_muted`)では足りない**——10px の字で
/// `text_ink` との差は明度で 2 割しかない。行の面に溶かして**半分以上沈める**。
const SUNK_ALPHA: f32 = 0.42;

/// 沈める。**「もう追わなくていい」を言う唯一の手つき**——タスクの待ちの行は
/// 全部同じ深さで沈める。パネルごとに別の濃さを持たせない。
#[must_use]
pub fn sunk(color: Color) -> Color {
    with_alpha(color, SUNK_ALPHA)
}

// ── §2-c 意味色(1色1義)──────────────────────────────────────────────
//
// 装飾には使わない。ここに無い意味へアクセントを足したくなったら、まず
// 文字の階層と面の高度で表せないかを疑う。

/// 稼働中。セッションの印・サブエージェント名。
#[must_use]
pub fn accent_active() -> Color {
    green()
}

/// 利用者の対応待ち。注意を向けてほしいもの・注記。
#[must_use]
pub fn accent_attention() -> Color {
    yellow()
}

/// 異常・失敗・破壊的な操作。
#[must_use]
pub fn accent_alert() -> Color {
    red()
}

/// いま入力を受けている場所。フォーカス枠と、その明滅。
#[must_use]
pub fn accent_focus() -> Color {
    blue()
}

// ── §2-c' 暦の曜日 ──────────────────────────────────────────────────────

/// 暦。日曜と祝日の日付。
#[must_use]
pub fn cal_holiday() -> Color {
    flamingo()
}

/// 暦。土曜の日付。
#[must_use]
pub fn cal_saturday() -> Color {
    blue()
}

// ── §2-d 寸法 ────────────────────────────────────────────────────────────

/// パネル・中央の枠の角。**角は立てる**——パネルは窓の端まで線で接するので、
/// 丸めると線の交わる所に欠けが出る。ブラウザの頁の角(`browser.rs` の `raise`)も
/// ここを読むので一緒に立つ。
#[must_use]
pub const fn radius_card() -> f32 {
    0.0
}

/// カードの内側にある小さいもの——行・ボタン・チップ・入力欄。
#[must_use]
pub const fn radius_control() -> f32 {
    4.0
}

// ── §2-e パネルのカード ─────────────────────────────────────────────────────

/// パネルのカード1枚ぶんの見た目。**パネルごとに組み直さない。**
#[must_use]
pub fn card_style() -> container::Style {
    container::Style {
        text_color: Some(text_primary()),
        background: Some(Background::Color(surface_card())),
        border: Border {
            radius: radius_card().into(),
            ..Border::default()
        },
        ..Default::default()
    }
}

/// 端末のパネル。器と端末の地は同じ色(`terminal::terminal_palette` の `background`)。
#[must_use]
pub fn terminal_seat_style() -> container::Style {
    card_style()
}

// ── §3 model / effort ────────────────────────────────────────────────────
// Claude Code の statusline でよく使われる Catppuccin Mocha の色。

pub const MOCHA_FABLE: Color = Color::from_rgb8(0xf5, 0xe0, 0xdc);
pub const MOCHA_OPUS: Color = Color::from_rgb8(0xf2, 0xcd, 0xcd);
pub const MOCHA_SONNET: Color = Color::from_rgb8(0xf5, 0xc2, 0xe7);
pub const MOCHA_DEFAULT: Color = Color::from_rgb8(0x93, 0x99, 0xb2);

#[must_use]
pub fn model(name: &str) -> Color {
    match name.to_ascii_lowercase().as_str() {
        "fable" => MOCHA_FABLE,
        "opus" => MOCHA_OPUS,
        "sonnet" => MOCHA_SONNET,
        _ => MOCHA_DEFAULT,
    }
}

#[must_use]
pub fn effort(name: &str) -> Color {
    match name.to_ascii_lowercase().as_str() {
        "low" => Color::from_rgb8(0x6c, 0x70, 0x86),
        "medium" => Color::from_rgb8(0x89, 0xb4, 0xfa),
        "high" => Color::from_rgb8(0x89, 0xdc, 0xeb),
        "xhigh" => Color::from_rgb8(0x94, 0xe2, 0xd5),
        "max" => Color::from_rgb8(0xa6, 0xe3, 0xa1),
        _ => MOCHA_DEFAULT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_and_effort_reuse_the_existing_mocha_colors() {
        assert_eq!(model("opus"), Color::from_rgb8(0xf2, 0xcd, 0xcd));
        assert_eq!(model("SONNET"), Color::from_rgb8(0xf5, 0xc2, 0xe7));
        assert_eq!(model("unknown"), MOCHA_DEFAULT);
        assert_eq!(effort("high"), Color::from_rgb8(0x89, 0xdc, 0xeb));
        assert_eq!(effort("max"), Color::from_rgb8(0xa6, 0xe3, 0xa1));
    }

    /// 窓の地とパネルは同じ1枚の画面。境は線が引く。
    #[test]
    fn the_window_and_the_panels_are_one_screen() {
        assert_eq!(surface_window(), surface_card());
        let screen = surface_card();
        let line = surface_line();
        assert!(line.r > screen.r && line.g > screen.g && line.b > screen.b);
        assert!(line.r < surface_raised().r, "線が浮く面より明るいと格子が立つ");
    }

    /// 面の階段は暗い順に1段ずつ並ぶ。隣り合う面が同じ色になると高度差が消える。
    #[test]
    fn the_ladder_gets_lighter_one_step_at_a_time() {
        let ladder = [surface_card(), surface_raised(), surface_active()];
        for pair in ladder.windows(2) {
            let (lower, upper) = (pair[0], pair[1]);
            assert!(
                lower.r < upper.r && lower.g < upper.g && lower.b < upper.b,
                "面の階段が逆転している: {lower:?} -> {upper:?}"
            );
        }
    }

    /// 文字は明るい順に落ちる。
    #[test]
    fn the_text_ladder_gets_dimmer_one_step_at_a_time() {
        let ladder = [text_primary(), text_secondary(), text_muted(), text_faint()];
        for pair in ladder.windows(2) {
            let (brighter, dimmer) = (pair[0], pair[1]);
            assert!(
                brighter.r > dimmer.r && brighter.g > dimmer.g && brighter.b > dimmer.b,
                "文字の階層が逆転している: {brighter:?} -> {dimmer:?}"
            );
        }
    }

    /// 中立色は配色の色みを持つ。無彩色グレー(R=G=B)を混ぜない歯止め。
    #[test]
    fn the_neutrals_are_never_plain_gray() {
        for theme in Theme::ALL {
            set_test_theme(Some(theme));
            for surface in [surface_card(), surface_line(), surface0(), surface1()] {
                assert!(
                    surface.r != surface.g || surface.g != surface.b,
                    "{theme:?} に無彩色の面: {surface:?}"
                );
            }
        }
        set_test_theme(None);
    }

    /// どの配色でも、面の階段と文字の階層が同じ順に並び、線は画面と浮く面の間に来る。
    #[test]
    fn every_theme_keeps_the_ladders_in_order() {
        let sum = |c: Color| c.r + c.g + c.b;
        for theme in Theme::ALL {
            set_test_theme(Some(theme));
            assert_eq!(surface_window(), surface_card(), "{theme:?}");
            assert!(sum(surface_card()) < sum(surface_line()), "{theme:?}");
            assert!(sum(surface_line()) < sum(surface_raised()), "{theme:?}");
            assert!(sum(surface_raised()) < sum(surface_active()), "{theme:?}");
            let text = [text_primary(), text_secondary(), text_muted(), text_faint()];
            for pair in text.windows(2) {
                assert!(sum(pair[0]) > sum(pair[1]), "{theme:?} の文字の階層: {pair:?}");
            }
            assert!(sum(text_lit()) > sum(text_primary()), "{theme:?}");
        }
        set_test_theme(None);
    }

    #[test]
    fn theme_keys_round_trip() {
        for theme in Theme::ALL {
            assert_eq!(Theme::from_key(theme.key()), Some(theme));
        }
        assert_eq!(Theme::from_key("solarized"), None);
    }

    /// 意味色は4つとも別の色であること。
    #[test]
    fn the_four_accents_never_collapse_into_one() {
        let accents = [
            accent_active(),
            accent_attention(),
            accent_alert(),
            accent_focus(),
        ];
        for (index, one) in accents.iter().enumerate() {
            for other in &accents[index + 1..] {
                assert_ne!(one, other);
            }
        }
    }
}
