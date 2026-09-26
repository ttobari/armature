use std::path::{Path, PathBuf};

pub const FAMILY: &str = "Moralerspace Argon";
pub const PRIMARY: iced::Font = iced::Font::with_name(FAMILY);
pub const TERMINAL: iced::Font = PRIMARY;

/// 時計のパネルの字(2026-09-05)。スイスの時計各社が使う Helvetica(Neue・Bold)。`load_fonts` が
/// macOS 同梱の HelveticaNeue.ttc を登録する
pub const CLOCK_HELVETICA: iced::Font = iced::Font {
    family: iced::font::Family::Name("Helvetica Neue"),
    weight: iced::font::Weight::Bold,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// 暦の今日。細字の格子の中で、今日の数字だけを1段起こして光らせる。
pub const CALENDAR_TODAY: iced::Font = iced::Font {
    family: iced::font::Family::Name("Helvetica Neue"),
    weight: iced::font::Weight::Medium,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// 脇のパネルの題(タスク・セッション)。ヒラギノ角ゴシックは和文と欧文を1本で持つので、
/// 字ごとの代替が起きない——SF Pro を頭に置くと漢字だけが代替へ落ち、簡体字の字形になる。
/// Light は macOS の和文の標準の太さ(W3)に当たる。数字・ID・端末は等幅のまま。
pub const UI: iced::Font = iced::Font {
    family: iced::font::Family::Name("Hiragino Sans"),
    weight: iced::font::Weight::Light,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// [`UI`] の太字(W6)。束の見出しと、選んでいる行。
pub const UI_STRONG: iced::Font = iced::Font {
    family: iced::font::Family::Name("Hiragino Sans"),
    weight: iced::font::Weight::Semibold,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// 暦の数字。**時計と同じ Helvetica Neue の細い側**——時計は
/// 段の主役なので Bold だが、暦は 11px の数字が 28 桝並ぶので、同じ太さだと格子が黒く潰れる。
/// 家族が同じなら「時計と揃っている」は保たれる。
pub const CALENDAR_DIGITS: iced::Font = iced::Font {
    family: iced::font::Family::Name("Helvetica Neue"),
    weight: iced::font::Weight::Light,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

const FILES: [&str; 4] = [
    "MoralerspaceArgon-Regular.ttf",
    "MoralerspaceArgon-Bold.ttf",
    "MoralerspaceArgon-Italic.ttf",
    "MoralerspaceArgon-BoldItalic.ttf",
];

/// 画面全体の拡大率の既定。
pub const DEFAULT_SCALE: f32 = 1.0;
/// これ以上縮めるとパネルの見出しと罫線が潰れて読めなくなる。
pub const MIN_SCALE: f32 = 0.7;
/// これ以上広げると左右のパネルに一行も収まらなくなる。
pub const MAX_SCALE: f32 = 1.8;
/// ⌘+ / ⌘- の一刻み。
const SCALE_STEP: f32 = 0.1;

const SYSTEM_FALLBACKS: [&str; 2] = [
    // Claude Code の返答頭 `⏺` を Apple Color Emoji へ逃がさず、文字記号で描く。
    "/System/Library/Fonts/Supplemental/STIXTwoMath.otf",
    "/System/Library/Fonts/Menlo.ttc",
];
const USER_FALLBACK: &str = "JetBrainsMonoNerdFontMono-Regular.ttf";

/// 端末を立てるのに要る分だけ(Regular・Bold と予備)。
///
/// 起動時に 4 本(各 8MB)を全部登録すると主線で 0.7 秒掛かる(2026-08-30 実測)。
/// Italic 2 本は [`load_late`] で端末が立ってから後追いする——それまで斜体は
/// Regular で出るだけで、パネルが止まるよりましだ。
#[must_use]
pub fn load_essential() -> Vec<Vec<u8>> {
    load_files(&FILES[..2])
}

/// 後追いで登録する分(Italic・BoldItalic)。
#[must_use]
pub fn load_late() -> Vec<Vec<u8>> {
    FILES[2..].iter().filter_map(|file| read_first(file)).collect()
}

/// 名前の書体を、置き場の優先順(同梱 → 利用者 → 機体)で最初に見つかった1本だけ読む。
/// 同じ書体を2度登録しない(1本 8MB ある)。
fn read_first(file: &str) -> Option<Vec<u8>> {
    font_dirs()
        .into_iter()
        .map(|dir| dir.join(file))
        .find_map(|path| std::fs::read(path).ok())
}

fn load_files(files: &[&str]) -> Vec<Vec<u8>> {
    let mut fonts: Vec<Vec<u8>> = files.iter().filter_map(|file| read_first(file)).collect();
    fonts.extend(
        SYSTEM_FALLBACKS
            .into_iter()
            .filter_map(|path| std::fs::read(path).ok()),
    );
    if let Some(bytes) = read_first(USER_FALLBACK) {
        fonts.push(bytes);
    }
    fonts
}

/// 拡大率を上下限へ収め、浮動小数の端数を落とす。
///
/// 刻みを足し引きし続けると `1.0999999` のような値が溜まる。記憶する値でも
/// あるので、ここで小数第2位へ丸めて桁を固定する。
#[must_use]
pub fn clamp_scale(value: f32) -> f32 {
    if !value.is_finite() {
        return DEFAULT_SCALE;
    }
    (value.clamp(MIN_SCALE, MAX_SCALE) * 100.0).round() / 100.0
}

/// 現在の拡大率から `steps` 刻みだけ動かした値。上下限で頭打ちになる。
#[must_use]
pub fn stepped_scale(current: f32, steps: i32) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let delta = SCALE_STEP * steps as f32;
    clamp_scale(clamp_scale(current) + delta)
}

/// 前回の窓が使っていた拡大率。無ければ既定。
#[must_use]
pub fn load_scale() -> f32 {
    std::fs::read_to_string(scale_path())
        .ok()
        .and_then(|text| text.trim().parse::<f32>().ok())
        .map_or(DEFAULT_SCALE, clamp_scale)
}

/// 拡大率を次の窓へ持ち越す。書けなくても画面は動くので失敗は黙って捨てる。
pub fn save_scale(value: f32) {
    let path = scale_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, format!("{:.2}\n", clamp_scale(value)));
}

fn scale_path() -> PathBuf {
    armature_core::geo::state_dir().join("armature-scale")
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // アプリ束に同梱した書体を最初に見る(`Contents/Resources/fonts`)。利用者の機体に
    // Moralerspace が入っていなくても、ダウンロードしたまま同じ字で立つ。
    if let Some(bundled) = bundled_font_dir() {
        dirs.push(bundled);
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join("Library/Fonts"));
    }
    dirs.push(PathBuf::from("/Library/Fonts"));
    dirs.push(PathBuf::from("/System/Library/Fonts"));
    dirs
}

/// 実体(`…/X.app/Contents/MacOS/<実体>`)から見た同梱の書体の置き場。束の外で
/// 起こしたとき(`cargo run` など)は無い。
fn bundled_font_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let contents = exe.parent()?.parent()?;
    let dir = contents.join("Resources/fonts");
    dir.is_dir().then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_stops_at_the_upper_and_lower_bound() {
        let mut widest = DEFAULT_SCALE;
        for _ in 0..64 {
            widest = stepped_scale(widest, 1);
        }
        assert!((widest - MAX_SCALE).abs() < 1e-6, "頭打ちが効いていない");

        let mut narrowest = DEFAULT_SCALE;
        for _ in 0..64 {
            narrowest = stepped_scale(narrowest, -1);
        }
        assert!((narrowest - MIN_SCALE).abs() < 1e-6, "底が効いていない");

        // 刻みを往復しても端数が溜まらない。
        assert!((stepped_scale(stepped_scale(DEFAULT_SCALE, 1), -1) - DEFAULT_SCALE).abs() < 1e-6);
        // 壊れた記憶(NaN・範囲外)は既定と上下限へ倒す。
        assert!((clamp_scale(f32::NAN) - DEFAULT_SCALE).abs() < 1e-6);
        assert!((clamp_scale(99.0) - MAX_SCALE).abs() < 1e-6);
        assert!((clamp_scale(0.01) - MIN_SCALE).abs() < 1e-6);
    }

    /// 暦の数字は時計と同じ家族(Helvetica Neue)。太さだけ落とす
    /// ——別の家族に振ると「時計と揃える」の指示(2026-09-07)が崩れる。
    #[test]
    fn the_calendar_digits_share_the_clock_family() {
        assert_eq!(CALENDAR_DIGITS.family, CLOCK_HELVETICA.family);
        assert_ne!(CALENDAR_DIGITS.weight, CLOCK_HELVETICA.weight);
    }

    #[test]
    fn claude_symbols_have_a_monochrome_fallback_before_emoji() {
        assert_eq!(
            SYSTEM_FALLBACKS[0],
            "/System/Library/Fonts/Supplemental/STIXTwoMath.otf"
        );
        assert!(Path::new(SYSTEM_FALLBACKS[0]).is_file());
    }
}
