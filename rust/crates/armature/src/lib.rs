//! Armature: a plain canvas for running Claude Code from your tasks, on the Mac.
//!
//! The app is this library; the `armature` binary only calls [`App::run`]. To add panels of your
//! own without touching this repository, make a crate that depends on `armature`, implement
//! [`Panel`] and run `App::new().panel(Mine::new()).run()` from its `main` (see `panel.rs` and
//! `CLAUDE.md`).

mod apple_translation;
mod appearance;
mod browser;
mod config;
mod ellipsized;
mod font;
mod glow;
mod help;
mod input_source;
#[allow(dead_code)]
mod md;
mod layout;
mod locale;
mod pacer;
pub mod palette;
pub mod panels;
mod panel;
mod quit;
mod resume;
mod session_ledger;
mod settings;
mod tap;
mod notes;
mod terminal;

use std::time::{Duration, Instant};

use std::path::{Path, PathBuf};

use iced::keyboard::{Key, Modifiers, key::Named};
use iced::widget::{
    Space, button, column, container, row, rule, stack, text,
    text_input,
};
use iced::{Alignment, Background, Border, Length, Size, Subscription, Task};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use palette::{
    accent_active, accent_alert, accent_focus,
    surface_card, surface_raised, surface_window, text_faint, text_muted,
    text_primary,
};

/// タブ列挙(約20ms・定コスト)だけで一覧を写し直す間隔。
///
/// 名簿(15秒)より細かく回す理由は、**窓の外から閉じられたタブ**——save/done の
/// Stop フックの `kill-pane`、claude 自身の終了——を拾うため。これが無いと左の一覧に
/// 死んだ行が15秒残り、台帳(⌘⇧T の戻し先)も閉じた刻を刻めない。
const TAB_SYNC: Duration = Duration::from_secs(2);
/// 窓の実寸を測り直す間隔。測る→揃え直す の往復が用事 2 件=コマ 2 枚になるので、
/// 毎秒ではなく 5 秒に 1 度(2026-09-05)。窓の大きさが変わった瞬間は Resized が別に来る。
const WINDOW_MEASURE: Duration = Duration::from_secs(5);
/// Ctrl+V の生バイト。Claude Code はこれを合図にクリップボードの画像を読む。
const CTRL_V: u8 = 0x16;

/// 本体の知らせが右の列の下に残る長さ。
const STATUS_LIFETIME: Duration = Duration::from_secs(5);

/// パネルの境の線の太さ(論理px。Retina で2画素)。
///
/// **窓は1枚の画面で、パネルは隙間なく接する。**列と列・パネルとパネルの間に
/// 置くのはこの線1本だけで、窓の縁には何も置かない——パネルをカードとして浮かせ、
/// 隙間で区切ると、パネルの数だけ角丸の箱が並んで画面が騒がしくなる。
const LINE: f32 = 1.0;


const FOCUS_FLASH_TICKS: u8 = 8;
/// 列の比。左右は 20、中央は 60——**中央は視野の中央 30° に収まる幅**
/// (27型を 65cm から見て約 1500pt)。
const SIDE_SHARE: u16 = 20;
const CENTRE_SHARE: u16 = 60;

/// 窓の組み立ての寸法。**パネルの並びと、Iced の外に浮くもの(WebView)の矩形は、
/// 両方ともここから引く**——片方だけが並びを読むと、幅を変えた瞬間にブラウザだけ
/// 枠から外れる。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    pad_x: f32,
    pad_y: f32,
    gap_x: f32,
    gap_y: f32,
    /// 列の比。パネルが1つも載っていない列は 0(列ごと出さない)。
    left: u16,
    centre: u16,
    right: u16,
}

impl Geometry {
    /// 列の比の合計。
    fn shares(self) -> f64 {
        f64::from(self.left + self.centre + self.right)
    }

    /// 列と列の隙間の数(出している横の列の数だけ)。
    fn gaps(self) -> f64 {
        f64::from(u8::from(self.left > 0) + u8::from(self.right > 0))
    }

    /// パネルを並べられる横幅(窓の縁2つと列の隙間を除いた残り・論理px)。
    fn available(self, width: f32) -> f64 {
        (f64::from(width) - f64::from(self.pad_x) * 2.0 - f64::from(self.gap_x) * self.gaps())
            .max(0.0)
    }
}

/// いまの並びでの窓の寸法。パネルが1つも載っていない列は出さない。中央がそのぶん広がる。
fn geometry() -> Geometry {
    let panels = layout::current();
    Geometry {
        pad_x: 0.0,
        pad_y: 0.0,
        gap_x: LINE,
        gap_y: LINE,
        left: if panels.left.is_empty() { 0 } else { SIDE_SHARE },
        centre: CENTRE_SHARE,
        right: if panels.right.is_empty() { 0 } else { SIDE_SHARE },
    }
}

/// 窓の実寸(拡大率に依らず一定に保つ)。
const WINDOW_WIDTH: f32 = 1440.0;
const WINDOW_HEIGHT: f32 = 900.0;

/// 落ちたときの言い分を必ず file へ残す。
///
/// **AppKit の block の中で panic すると、Rust は巻き戻せずその場で abort する。**
/// そのとき標準エラーは誰も受けていないので、macOS のクラッシュ報告には
/// 「abort() called」しか残らない——2026-08-23 の落ち方がまさにそれで、
/// 何が起きたのか一行も分からなかった。panic フックは abort の前に走るので、
/// ここで書けば次からは理由が読める。
fn log_panics() {
    let path = armature_core::geo::state_dir().join("panic.log");
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let at = info
            .location()
            .map_or_else(|| "(unknown location)".to_string(), ToString::to_string);
        let what = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_string());
        let when = jiff::Zoned::now().strftime("%F %T");
        let trace = std::backtrace::Backtrace::force_capture();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write as _;
            let _ = writeln!(file, "── {when} ──\n{at}\n{what}\n{trace}");
        }
        previous(info);
    }));
}

/// Armature, ready to run. Add panels of your own with [`App::panel`], then call [`App::run`].
pub struct App {
    custom: Vec<Box<dyn panel::DynPanel>>,
    infos: Vec<layout::CustomInfo>,
    identity: Option<(String, String)>,
    /// The keys of the panels Armature put in ([`App::new`]) that yours have not replaced.
    defaults: Vec<&'static str>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Armature with the panels it comes with ([`crate::panels`]). Leave one out with
    /// [`App::without`], or replace it by adding yours under the same key.
    #[must_use]
    pub fn new() -> Self {
        Self::bare()
            .builtin(panels::BrowserTabs::new())
            .builtin(panels::Music::new())
            .builtin(panels::Sessions::new())
            .builtin(panels::Clock::new())
            .builtin(panels::Calendar::new())
            .builtin(panels::Tasks::new())
    }

    /// Armature with no panels at all: the terminal alone. Add the ones you want with
    /// [`App::panel`] (the built-in ones are in [`crate::panels`]).
    #[must_use]
    pub fn bare() -> Self {
        Self {
            custom: Vec::new(),
            infos: Vec::new(),
            identity: None,
            defaults: Vec::new(),
        }
    }

    /// Leaves out one of the panels Armature comes with (`"music"`, `"clock"`, …).
    #[must_use]
    pub fn without(mut self, key: &str) -> Self {
        if let Some(index) = self.infos.iter().position(|info| info.key == key)
            && self.defaults.contains(&key)
        {
            self.infos.remove(index);
            self.custom.remove(index);
            self.defaults.retain(|default| *default != key);
        }
        self
    }

    fn builtin<P: Panel>(mut self, panel: P) -> Self {
        let info = panel::info::<P>();
        self.defaults.push(info.key);
        self.infos.push(info);
        self.custom.push(Box::new(panel));
        self
    }

    /// Gives your build a name of its own, so it keeps its files apart from plain Armature's and
    /// the two can run side by side. Plain Armature is `("Armature", "armature")`.
    ///
    /// - `name`: the window's title and the task folder in your home (`~/<name>`, which also
    ///   holds `panels.conf`)
    /// - `slug` (lowercase letters, digits and `-`): the state folder
    ///   (`~/Library/Application Support/<slug>`) and Armature's own tmux server (`tmux -L <slug>`)
    ///
    /// Without it your build shares all three with plain Armature. To share only the tasks, set
    /// `TASKS_HOME` in `<state folder>/config.env`.
    ///
    /// # Panics
    /// When `name` is empty, starts or ends with a space, or holds `/` or `:`, or `slug` is not
    /// lowercase letters, digits and `-`.
    #[must_use]
    pub fn name(mut self, name: &str, slug: &str) -> Self {
        assert!(
            armature_core::paths::is_valid_name(name),
            "app name {name:?} must be a folder name: not empty, no '/' or ':', no spaces at the ends"
        );
        assert!(
            armature_core::paths::is_valid_slug(slug),
            "app slug {slug:?} must be lowercase letters, digits and '-'"
        );
        self.identity = Some((name.to_string(), slug.to_string()));
        self
    }

    /// Adds a panel of your own (see [`Panel`]).
    ///
    /// A panel whose key is also a built-in panel's replaces that panel in your build (with a
    /// warning on standard error), so a panel Armature later builds in under the same name does not
    /// stop your build from starting.
    ///
    /// # Panics
    /// When its key is empty or not lowercase letters, digits and `-`, or another panel of yours
    /// has it already.
    #[must_use]
    pub fn panel<P: Panel>(mut self, panel: P) -> Self {
        let info = panel::info::<P>();
        assert!(
            !info.key.is_empty()
                && info
                    .key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "panel key {:?} must be lowercase letters, digits and '-'",
            info.key
        );
        // Armature's own panel with the same key: yours takes its place (and its spot in the columns).
        if let Some(index) = self.infos.iter().position(|other| other.key == info.key) {
            assert!(
                self.defaults.contains(&info.key),
                "panel key {:?} is already taken by another of your panels",
                info.key
            );
            self.defaults.retain(|default| *default != info.key);
            self.infos[index] = info;
            self.custom[index] = Box::new(panel);
            return self;
        }
        self.infos.push(info);
        self.custom.push(Box::new(panel));
        self
    }

    /// Opens the window and runs until it is closed.
    ///
    /// # Errors
    /// When the window or the GPU cannot be set up.
    pub fn run(self) -> Result<(), Error> {
        launch(self)
    }
}


/// What [`App::run`] fails with.
pub type Error = iced::Error;

pub use armature_core::{lang, paths, tr};
pub use iced;
pub use iced::Element;
pub use layout::Side;
pub use panel::{BrowserTab, Host, KeyPress, Notice, Panel, Tab};

fn launch(app: App) -> Result<(), Error> {
    // 身元は置き場を読むより前に据える(panic.log も状態の置き場に書く)。
    if let Some((name, slug)) = &app.identity {
        armature_core::paths::set_identity(name, slug);
    }
    log_panics();
    align_app_paths();
    locale::init();
    appearance::init();
    // 見本のタスクは言語が決まってから書く(前は先に書いていて、日本語の Mac でも英語になった)。
    seed_first_task();
    // 書体は Cockpit::new が返す Task で登録する(application::font だと完了を
    // 受け取れず、端末がセル寸法を測る時刻と競う)。
    // 窓の寸法は拡大率を掛けてから winit へ渡るので、記憶した拡大率で開くときに
    // 窓そのものが太らないよう、ここで割り戻して実寸を 1440×900 に据える。
    let scale = font::load_scale();
    let place = load_window_place();
    layout::register(app.infos);
    // 外から足したパネル。窓を起こすときに1度だけ `Cockpit` へ渡す。
    let custom = std::cell::RefCell::new(app.custom);
    iced::application(
        move || Cockpit::new(custom.take()),
        Cockpit::update,
        Cockpit::view,
    )
    .title(|_: &Cockpit| armature_core::paths::name().to_owned())
    .subscription(Cockpit::subscription)
    .theme(|_: &Cockpit| iced::Theme::Dark)
    .scale_factor(Cockpit::scale_factor)
    .settings(iced::Settings {
        default_font: font::PRIMARY,
        antialiasing: true,
        ..iced::Settings::default()
    })
    .window(iced::window::Settings {
        size: Size::new(WINDOW_WIDTH / scale, WINDOW_HEIGHT / scale),
        // 居場所の控えがあればそこへ。全画面はこの窓が乗った画面で起きるので、
        // 位置を捨てると主画面へ引っ越してしまう。
        position: place
            .position
            .map_or(iced::window::Position::Centered, iced::window::Position::Specific),
        // **ここで全画面にはしない。** iced は窓をこしらえるときの全画面を
        // `Borderless(None)` で組むので、どの画面に置いても主画面で全画面に
        // なる。姿は窓が立ってから `set_mode` で入れる——あちらは `current_monitor` を
        // 使うので、控えた位置の画面でそのまま全画面になる。
        // WKWebView の破棄を待って赤ボタンが長時間固まる経路を避けるため、
        // CloseRequested は update 側で同期保存してから即時終了する。
        exit_on_close_request: false,
        ..iced::window::Settings::default()
    })
    .run()
}

/// 置き場をこのアプリ専用に揃える。**起動の最初に1度だけ。**
fn align_app_paths() {
    let state = armature_core::paths::default_state();
    let _ = std::fs::create_dir_all(&state);
    // 設定でタスクの置き場を替えていれば、そちらを正本にする(`TASKS_HOME`)。
    // 設定ファイルは状態の置き場(`paths::set` 前でも既定を指す)にある。
    let tasks_home = config::read_value("TASKS_HOME")
        .map(|raw| terminal::expand_home(&raw))
        .unwrap_or_else(armature_core::paths::default_tasks_home);
    armature_core::paths::set(tasks_home, state);
    let _ = std::fs::create_dir_all(armature_core::paths::tasks_dir());
}

/// 初めて窓が開いたときだけ、同梱の「はじめに」の頁の URL を返す(印は `welcomed`)。
/// 束の外で起こしたとき(頁が無い)は何もしない。
fn take_first_welcome() -> Option<String> {
    let mark = armature_core::paths::state().join("welcomed");
    if mark.exists() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let page = exe
        .parent()?
        .parent()?
        .join(tr!("Resources/welcome.html", "Resources/welcome-ja.html"));
    let html = std::fs::read_to_string(&page).ok()?;
    // 同梱の頁は素の Armature の置き場(~/Armature)で書いてある。名前を替えた版でも
    // 正しいフォルダを指すよう、置き場と名前を差し込んだ写しを状態の置き場に書いて開く。
    let home = std::env::var("HOME").unwrap_or_default();
    let html = personalize_welcome(
        &html,
        &shown_path(&armature_core::paths::tasks_home(), &home),
        armature_core::paths::name(),
    );
    let copy = armature_core::paths::state().join(page.file_name()?);
    std::fs::write(&copy, html).ok()?;
    let _ = std::fs::write(&mark, "");
    Some(format!("file://{}", copy.to_string_lossy().replace(' ', "%20")))
}

/// `/Users/someone/Armature` → `~/Armature`(ホームの下でなければそのまま)。
fn shown_path(path: &std::path::Path, home: &str) -> String {
    let path = path.to_string_lossy();
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => path.into_owned(),
    }
}

/// はじめにの頁の `~/Armature` と題の `Armature` を、このアプリの置き場と名前に替える。
fn personalize_welcome(html: &str, tasks_home: &str, name: &str) -> String {
    html.replace("~/Armature", &browser::html_escape(tasks_home))
        .replace("— Armature</title>", &format!("— {}</title>", browser::html_escape(name)))
}

/// Mac の既定のブラウザで開く(ブラウザのパネルを外しているとき)。`--` の後ろに置くので、
/// `-` で始まる綴りも `open` の引数としては読まれない。
fn open_in_default_browser(url: &str) -> Result<(), String> {
    let mut command = std::process::Command::new("/usr/bin/open");
    command.arg("--").arg(url);
    armature_core::proc::spawn_reaped_quiet(command).map(drop).map_err(|error| {
        tr!(
            format!("Can't open the default browser: {error}"),
            format!("既定のブラウザを起こせない: {error}"),
        )
        .to_string()
    })
}

/// 初めて起こしたときだけ、見本のタスクを1枚置く。**消されたら二度と置かない**
/// (印は状態の置き場の `seeded`)。
fn seed_first_task() {
    let mark = armature_core::paths::state().join("seeded");
    if mark.exists() {
        return;
    }
    let dir = armature_core::paths::tasks_dir();
    let empty = std::fs::read_dir(&dir).map_or(true, |entries| {
        !entries
            .flatten()
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
    });
    if empty {
        let today = Timestamp::now().to_zoned(TimeZone::system()).date();
        let _ = std::fs::write(dir.join("1.md"), first_task_text(today));
    }
    let _ = std::fs::write(mark, "");
}

fn first_task_text(today: Date) -> String {
    let dir = armature_core::paths::tasks_dir();
    tr!(
        format!(
            "---\nid: 1\nparent: \nstatus: active\ncreated: {today}\nupdated: {today}\n---\n\n\
             # Getting started\n\n\
             A task for trying out the app.\n\n\
             - Select this row in the task list and press ⌃L (or ⇧L): a Claude session for this task opens in the center\n\
             - Claude reads this file to start, and writes \"Where it stands\" and \"Next steps\" back at break points\n\
             - To add a task, ask Claude (\"Add a task to …\"). Each task is one file in {dir}\n\
             - Press d twice in the list to mark a task done\n\
             - ⌘/ lists every key\n\
             - ⌘, sets the language and the color theme. To move panels, ask Claude\n\n\
             ## Where it stands\n\n\
             ## Next steps\n\n\
             Ask Claude: \"Explain briefly how to use this app.\"\n",
            dir = dir.display(),
        ),
        format!(
            "---\nID: 1\n親: \n状態: 進行\n作成日: {today}\n更新日: {today}\n---\n\n\
             # はじめに\n\n\
             このアプリを試すためのタスク。\n\n\
             - タスクの一覧でこの行を選び、⌃L(または ⇧L)を押すと、このタスク用の Claude が中央に立ち上がる\n\
             - Claude はこのファイルを読んで始め、区切りで「現在地」と「次にやること」を書き戻す\n\
             - 新しいタスクは Claude に「〇〇をタスクに足して」と頼む。ファイルは {dir} に1枚ずつ置かれる\n\
             - 終わったタスクは、一覧で d を2回押すと完了になる\n\
             - キーの一覧は ⌘/\n\
             - ⌘, で言語と配色を変えられる。パネルの並びは Claude に頼む\n\n\
             ## 現在地\n\n\
             ## 次にやること\n\n\
             Claude に「このアプリの使い方を短く説明して」と頼んでみる。\n",
            dir = dir.display(),
        ),
    )
}

struct Cockpit {
    terminal: Option<iced_term::Terminal>,
    /// 窓が開いたか。端末はこれと書体の到着が揃ってから起こす。
    window_opened: bool,
    /// 端末の生成に実際に失敗したときだけ入る。待ち状態とは区別する。
    terminal_error: Option<String>,
    /// 書体が font system へ届いたか。
    fonts_ready: bool,
    /// 右の一覧が鍵を受けているか。中央の端末と取り合う。
    right_focus: bool,
    /// 右の一覧の鍵を受ける見えない入力欄の中身。打たれたら捨てる。
    key_sink_text: String,
    right_focus_id: iced::widget::Id,
    /// どのパネルをどの列の何番目に出すか(`layout.rs`・`panels.conf`)。
    panels: layout::Layout,
    /// 外から足したパネル(`panel.rs`)。並びの `PanelId(i)` の i 番目。
    custom: Vec<Box<dyn panel::DynPanel>>,
    /// 鍵を持っているパネル(`custom` の添字)。
    keys_panel: Option<u16>,
    /// 最後に鍵を持っていたパネル。⌘L はここへ戻す。
    last_keys_panel: Option<u16>,
    /// 端末のタブ(tmux の窓)。パネルへは `Notice::Tabs` で配る。
    tabs: Vec<armature_core::monitor::LocalTab>,
    /// パネルへ最後に配ったタブの写し。変わったときだけ配り直す。
    told_tabs: Option<Vec<armature_core::monitor::LocalTab>>,
    /// パネルへ最後に配ったブラウザのタブの写し。
    told_browser: Option<(Vec<panel::BrowserTab>, usize, bool)>,
    /// `panels.conf` の刻み。手で・Claude に書き換えられたら並べ直す。
    panels_stamp: Option<(std::time::SystemTime, u64)>,
    /// 本体からの短い知らせ(タブを起こせなかった等)と、出した刻。数秒で消える。
    status: Option<(String, Instant)>,
    /// 中央を借りているパネルに閉じてもらう印(ほかの画面が中央を取った)。
    dismiss_pending: bool,
    /// タブ列挙の世代と、最後に確認できたタブ状態。
    tab_sync: TabSync,
    /// 端末の準備(configure_tmux)が裏線で走っている間 true。
    terminal_preparing: bool,
    browser_mode: bool,
    browser_tabs: Vec<browser::TabInfo>,
    browser_active: usize,
    browser_revived: bool,
    browser_error: Option<String>,
    /// いまブラウザに出している資料。書き替わったら載せ替えるために控える。
    watched_file: Option<WatchedFile>,
    help_visible: bool,
    help_restore_browser: bool,
    /// 設定のパネル(言語とパネルの並び)。中央に出ている間だけキーを受ける。
    settings_visible: bool,
    settings_restore_browser: bool,
    focus_flash: Option<FocusFlash>,
    window_size: Size,
    /// 画面全体の拡大率(⌘+ / ⌘- / ⌘0)。窓の実寸ではなく描画の倍率。
    scale: f32,
    startup_binary: Option<BinaryFingerprint>,
    restart_available: bool,
    restart_error: Option<String>,
    /// ▶ Restart を押してから、次の窓が開いたのを確かめて落ちるまでの間 true。
    /// 二度押しで窓を2枚起こさないための錠。
    restart_pending: bool,
    /// アドレス欄に出している字。頁が動けば追随し、打ち始めたら手を引く。
    address: String,
    address_id: iced::widget::Id,
    /// アドレス欄を利用者が触っている間。頁側の URL で上書きしない印。
    address_editing: bool,
    /// 前の窓が全画面のまま ▶ Restart を押していたか。窓が開いた1度だけ使う。
    restore_fullscreen: bool,
    /// 最後に窓の姿を控えた刻。全画面へ出入りする間、伸縮は何十回も来る。
    mode_noted_at: Option<Instant>,
    /// タブ列挙で一覧を写し直した刻。`TAB_SYNC` ごとに回す。
    last_tab_sync: Instant,
    last_window_measure: Instant,
}

#[derive(Clone, Debug)]
enum Message {
    /// 設定のパネルから出た合図(言語・パネルの並び)。
    Settings(settings::Message),
    /// 外から足したパネル(`panel.rs`)の合図。`PanelId::Custom` の番号つき。
    Custom(u16, panel::Payload),
    /// パネルが押された。鍵を受けるパネル(`Panel::KEYS`)なら鍵を渡す。
    PanelPressed(u16),
    /// 本体の ⌘ キーでない打鍵。鍵を持っているパネルへ(`Panel::key`)。
    PanelKey(panel::KeyPress),
    /// 自作パネルの画面(欄・釦)から来た知らせ。一覧の鍵を下ろしてから [`Message::Custom`] へ。
    CustomInput(u16, panel::Payload),
    Tick,
    BrowserPoll,
    Terminal(iced_term::Event),
    Shortcut(Shortcut),
    /// 見えない入力欄に字が入った(捨てるだけ)。
    KeySinkInput,
    /// ⌘V を入力欄が自分で貼った(Captured)。一覧の鍵受けなら端末へも回す。
    FieldPaste,
    /// 右の一覧・中央の設定を動かす素のキー。
    /// `x`。タスク一覧の行を掴む/離す。
    /// `d` の1打。2打そろって初めて完了へ倒す。
    /// パネルが頼んだ Claude のタブが立った(または立たなかった)。
    TabStarted(Result<String, String>),
    /// ⌘ と、どれかのパネルが名乗った字(`Panel::SHORTCUT`)。
    PanelShortcut(char),
    /// クリップボードから読み出した字。無ければ何もしない。
    PasteText(Option<String>),
    BrowserState(Result<browser::Snapshot, String>),
    BrowserChanged(Result<(), String>),
    /// 窓の実寸の測り直し。`Resized` を取り落とした回をここで拾う。
    WindowMeasured(Size),
    BrowserTranslation(browser::TranslationReply),
    Restart,
    RestartWithMode(iced::window::Mode),
    /// 窓がいまどんな姿でどこに居るか。控えて、次に起こすときの姿と画面にする。
    WindowPlaceNoted(iced::window::Mode, Option<iced::Point>),
    /// 次の窓が開いた(か、待ちきれなかった)。ここで初めて古い窓が落ちる。
    RestartHandoffDone,
    AddressChanged(String),
    AddressSubmitted,
    BrowserHistory(i32),
    BrowserReload,
    BrowserOpenExternally,
    SessionPressed(String),
    /// 窓順の入れ替えが済んだ。タブ列挙を採り直す。
    SessionsReordered,
    BrowserTabPressed(usize),
    /// タブ一覧が送られた。隠れた行の在り処を印に出すため控える。
    NativeShortcutsReady,
    FontsLoaded,
    /// タブ列挙が裏線から届いた。
    TabsSynced(u64, Vec<armature_core::monitor::LocalTab>),
    /// 端末の起動口(tmux の準備)が裏線で整った。
    TerminalPrepared(terminal::Launch),
    /// 後追いの書体(Italic)が登録し終えた。
    LateFontsLoaded,
    WindowEvent(iced::window::Event),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shortcut {
    Claude,
    Shell,
    /// 閉じた Claude タブを会話ごと戻す。
    Reopen,
    Next,
    Previous,
    Browser,
    Center,
    Right,
    Close,
    Help,
    /// 設定(⌘ ,)。
    Appearance,
    /// 窓ごと起こし直す(⇧⌘ R)。▶ Restart の札と同じ道。
    Restart,
    /// 中央の端末へ鍵を渡す(⌘ ⏎)。頁が出ていれば畳んでから渡す。
    Terminal,
    Paste,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusPane {
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FocusFlash {
    pane: FocusPane,
    ticks_remaining: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BinaryFingerprint {
    path: std::path::PathBuf,
    device: u64,
    inode: u64,
    len: u64,
    modified: Option<(u64, u32)>,
}

impl Cockpit {
    /// 画面全体の拡大率。Iced はこの値で論理座標を割り、パネル・端末・文字が
    /// まとめて拡縮する。
    fn scale_factor(&self) -> f32 {
        self.scale
    }

    /// 拡大率を差し替える。
    ///
    /// 窓の実寸は変わらないので、変えるのは Iced が使う論理寸法のほうだけ。
    /// 拡大率が動いても winit は `Resized` を投げない(Iced は viewport を
    /// 差し替えるだけ)ので、`window_size` はここで割り戻す。中央の WebView は
    /// Iced の外に浮いていて自動では動かないため、枠を置き直す。
    fn apply_scale(&mut self, scale: f32) -> Task<Message> {
        let scale = font::clamp_scale(scale);
        if (scale - self.scale).abs() < f32::EPSILON {
            return Task::none();
        }
        let ratio = self.scale / scale;
        self.window_size = Size::new(
            self.window_size.width * ratio,
            self.window_size.height * ratio,
        );
        self.scale = scale;
        font::save_scale(scale);
        // パネルの文字と一緒に頁の字も動かす。矩形だけ直すと
        // 窓の字は育つのにブラウザの本文だけ据え置きになる。
        Task::batch([resize_browser(self.window_size, scale), zoom_browser(scale)])
    }

    fn new(custom: Vec<Box<dyn panel::DynPanel>>) -> (Self, Task<Message>) {
        let now = Instant::now();
        // 前の窓からの申し送り。**1度しか読まない**——残すと、次に普通に
        // 起こしたときまで全画面や巻を引きずる。
        let restart_note = take_restart_note();
        let scale = font::load_scale();
        let startup_binary = current_binary_fingerprint();
        let saved_browser = browser::load_saved();
        let panels = layout::Layout::load();
        // 最初の一度はファイルを置いておく——Claude に並べ替えを頼んだとき、見つけられるように。
        if !layout::Layout::path().exists() {
            panels.store();
        }
        layout::publish(&panels);
        let mut app = Self {
            // application::font が compositor へ登録されるのは boot 後。
            // 端末をここで作ると再起動時だけ代替フォントでセル幅を測るため、
            // Opened を受けてから初期化する。
            terminal: None,
            window_opened: false,
            terminal_error: None,
            fonts_ready: false,
            // 起動時は中央を端末のまま、右は ⌘R と同じタスク一覧で立てる。
            // 鍵の行き先は中央のままで、
            // 右へ移すのは ⌘R・⌘N・⌘L の打鍵——開いた直後の打鍵が Claude へ入る形は変えない。
            right_focus: false,
            key_sink_text: String::new(),
            right_focus_id: iced::widget::Id::unique(),
            panels,
            custom,
            keys_panel: None,
            last_keys_panel: None,
            tabs: Vec::new(),
            told_tabs: None,
            told_browser: None,
            panels_stamp: file_stamp(&layout::Layout::path()),
            status: None,
            dismiss_pending: false,
            tab_sync: TabSync::default(),
            terminal_preparing: false,
            browser_mode: false,
            browser_tabs: saved_browser.tabs,
            browser_active: saved_browser.active,
            browser_revived: false,
            browser_error: None,
            watched_file: None,
            help_visible: false,
            help_restore_browser: false,
            settings_visible: false,
            settings_restore_browser: false,
            focus_flash: None,
            window_size: Size::new(WINDOW_WIDTH / scale, WINDOW_HEIGHT / scale),
            scale,
            startup_binary,
            restart_available: false,
            restart_error: None,
            restart_pending: false,
            address: String::new(),
            address_id: iced::widget::Id::unique(),
            address_editing: false,
            restore_fullscreen: restart_note.fullscreen,
            mode_noted_at: None,
            last_tab_sync: now,
            last_window_measure: now,
        };
        // 端末のタブは最初の描画の前から取りに行く(パネルへは `Notice::Tabs` で届く)。
        let tabs = app.request_tab_sync();
        app.write_notes();
        (app, Task::batch([tabs, load_fonts()]))
    }


    /// 用事を1つ捌き、パネルが知りたい状態が変わっていれば配る。
    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.step(message);
        Task::batch([task, self.tell_changes()])
    }

    fn step(&mut self, message: Message) -> Task<Message> {
        if center_yields_to(&message) {
            self.dismiss_pending = true;
        }
        match message {
            Message::Tick => {
                if self.status.as_ref().is_some_and(|(_, at)| at.elapsed() >= STATUS_LIFETIME) {
                    self.status = None;
                }
                self.reload_watched_file();
                let current_binary = current_binary_fingerprint();
                self.restart_available =
                    binary_was_replaced(self.startup_binary.as_ref(), current_binary.as_ref());
                let now = Instant::now();
                let mut tasks = Vec::new();
                // タブの並び(tmux の窓の列挙・軽い)を写し続ける。変われば `Notice::Tabs`。
                if now.duration_since(self.last_tab_sync) >= TAB_SYNC {
                    tasks.push(self.request_tab_sync());
                }
                // パネルの並びのファイルが書き換わっていたら並べ直す(設定の画面・手・Claude のどれでも)。
                let stamp = file_stamp(&layout::Layout::path());
                if stamp != self.panels_stamp {
                    self.panels_stamp = stamp;
                    let fresh = layout::Layout::load();
                    if fresh != self.panels {
                        self.panels = fresh;
                        layout::publish(&self.panels);
                        tasks.push(self.after_panels_changed());
                    }
                }
                // **窓の実寸は毎秒測り直す。**`Resized` の便を1回落とすだけで
                // WebView は古い枠のまま取り残される——Iced の外に浮いていて
                // 自分では追随しないので、パネルだけが正しく、頁が左上に小さく
                // 寄る。頁を1枚も抱えていないときは測らない。
                if !self.browser_tabs.is_empty()
                    && now.duration_since(self.last_window_measure) >= WINDOW_MEASURE
                {
                    self.last_window_measure = now;
                    tasks.push(measure_window());
                }
                Task::batch(tasks)
            }
            Message::CustomInput(index, payload) => {
                // 鍵を持っていないパネル(の欄)を触ったら鍵を下ろす。欄に打った字を、
                // 鍵を持っているパネルの操作にしない。
                if self.keys_panel != Some(index) {
                    self.right_focus = false;
                }
                self.update(Message::Custom(index, payload))
            }
            Message::Custom(index, payload) => {
                self.with_panel(index, |panel, host| panel.update(payload, host))
            }
            Message::PanelPressed(index) => self.give_keys(index),
            Message::PanelKey(key) => {
                // 設定の画面が中央に出ている間は、素のキーはまずそちらへ。L/H で言語、J/K で配色。
                if self.settings_visible
                    && let Some(message) = settings_key(&key)
                {
                    return self.update(Message::Settings(message));
                }
                // 中央を借りているパネルは Esc で閉じる。
                if key.key == Key::Named(Named::Escape) && self.panel_center_open() {
                    return self.dismiss_centers();
                }
                let Some(index) = self.keys_panel.filter(|_| self.right_focus) else {
                    return Task::none();
                };
                self.with_panel(index, |panel, host| panel.key(&key, host).unwrap_or_else(Task::none))
            }
            Message::Settings(message) => match message {
                settings::Message::Language(index) => {
                    locale::choose(armature_core::lang::Lang::from_index(index));
                    self.write_notes();
                    Task::none()
                }
                settings::Message::Theme(index) => {
                    appearance::choose(palette::Theme::ALL[index % palette::Theme::ALL.len()]);
                    // 端末の色表は作ったときに写し取っているので、送り直す。
                    if let Some(term) = self.terminal.as_mut() {
                        term.handle(iced_term::Command::ChangeTheme(Box::new(
                            terminal::terminal_palette(),
                        )));
                    }
                    Task::none()
                }
            },
            Message::BrowserPoll => {
                self.focus_flash = tick_focus_flash(self.focus_flash);
                Task::batch(
                    browser::take_requests()
                        .into_iter()
                        .map(|request| self.handle_browser_request(request)),
                )
            }
            Message::Terminal(iced_term::Event::BackendCall(_, command)) => {
                if terminal_command_claims_focus(&command) {
                    self.right_focus = false;
                }
                // ホイールは端末自身の巻物ではなくマウス報告として中へ渡す。
                // 端末側の巻物は tmux が全画面で描いている間ずっと空のまま。
                let command = match command {
                    iced_term::BackendCommand::Scroll(lines) => {
                        let report = terminal::wheel_report(lines);
                        if report.is_empty() {
                            return Task::none();
                        }
                        iced_term::BackendCommand::Write(report)
                    }
                    other => other,
                };
                let action = self
                    .terminal
                    .as_mut()
                    .map(|term| term.handle(iced_term::Command::ProxyToBackend(command)));
                if let Some(iced_term::actions::Action::OpenLink(url)) = action {
                    // ブラウザのパネルを外していれば、Mac の既定のブラウザで開く。
                    if !self.shows_key("browser") {
                        if url.starts_with("https://") || url.starts_with("http://") {
                            let _ = open_in_default_browser(&url);
                        }
                        return Task::none();
                    }
                    // **端末は前のまま。**頁は後ろで開いて札だけ増やす。
                    // 読んでいる途中に画面を奪われると、どこを読んでいたか見失う。
                    // 既に頁を見ているときは、そのまま新しい札を足すだけ。
                    self.right_focus = false;
                    self.browser_error = None;
                    let was_open = self.browser_mode;
                    let open =
                        open_browser(browser::Source::Url(url), self.window_size, self.scale);
                    if was_open {
                        return open;
                    }
                    self.browser_mode = false;
                    return open.chain(hide_browser()).chain(self.terminal_focus());
                }
                Task::none()
            }
            Message::Shortcut(shortcut) => {
                match shortcut {
                    Shortcut::Claude => {
                        if self.browser_mode {
                            return self.open_new_browser_tab();
                        }
                        // **戻り値を捨てない。** tmux も claude も見つからなければ
                        // 起こさずに false が返るだけで、黙っていると利用者からは
                        // 「押しても何も起きない」に見える(2026-08-31)。
                        self.tab_sync.invalidate();
                        if !terminal::open_tab(terminal::TabKind::Claude) {
                            self.say(tr!(
                                "Couldn't open a Claude tab (tmux or claude not found)",
                                "claude のタブを起こせなかった(tmux か claude が見つからない)",
                            ));
                            return Task::none();
                        }
                        // 起こした直後に一覧を取り直す。定期の同期を待つと、
                        // 起きているのに数秒出てこない(2026-08-31 の「起動したのに
                        // 出てこない」の正体)。
                        return self.request_tab_sync();
                    }
                    Shortcut::Reopen => {
                        // 押した時点で tmux を見て、消えている控えへ刻を打つ。
                        // 毎秒見張らないのは、そのたびにプロセスを起こしたくないため
                        // ——押すのは事故のあとだけなので、ここで見れば足りる。
                        session_ledger::sweep(&terminal::live_session_ids());
                        let Some(entry) = session_ledger::latest_closed() else {
                            return Task::none();
                        };
                        // 戻せたぶんの控えは reopen の中で生き返る扱いに直る。
                        self.tab_sync.invalidate();
                        let _ = terminal::reopen(&entry);
                        // 頁の上で押したときは、戻したタブが頁の裏に隠れないよう畳む。
                        if self.browser_mode {
                            self.browser_mode = false;
                            self.right_focus = false;
                            return Task::batch([hide_browser(), self.terminal_focus_only()]);
                        }
                        return Task::none();
                    }
                    Shortcut::Shell => {
                        self.tab_sync.invalidate();
                        if !terminal::open_tab(terminal::TabKind::Shell) {
                            self.say(tr!(
                                "Couldn't open a terminal tab (tmux not found)",
                                "端末のタブを起こせなかった(tmux が見つからない)",
                            ));
                            return Task::none();
                        }
                        // ⌘J のシェルは英字入力で開く。会話端末の入力復元は通さない。
                        input_source::to_ascii();
                        self.right_focus = false;
                        if self.browser_mode {
                            self.browser_mode = false;
                            self.right_focus = false;
                            return Task::batch([
                                hide_browser(),
                                self.terminal_focus_only(),
                                self.request_tab_sync(),
                            ]);
                        }
                        return Task::batch([self.terminal_focus_only(), self.request_tab_sync()]);
                    }
                    Shortcut::Next => {
                        if self.browser_mode {
                            return cycle_browser(1);
                        }
                        self.tab_sync.invalidate();
                        terminal::next_tab();
                    }
                    Shortcut::Previous => {
                        if self.browser_mode {
                            return cycle_browser(-1);
                        }
                        self.tab_sync.invalidate();
                        terminal::previous_tab();
                    }
                    Shortcut::Browser => {
                        if !self.shows_key("browser") {
                            return Task::none();
                        }
                        if self.browser_mode {
                            self.browser_mode = false;
                            return Task::batch([hide_browser(), self.terminal_focus()]);
                        }
                        if !self.browser_tabs.is_empty() {
                            self.dismiss_pending = true;
                            self.browser_mode = true;
                            self.right_focus = false;
                            return show_browser(self.window_size, self.scale);
                        }
                        return self.open_new_browser_tab();
                    }
                    Shortcut::Terminal => {
                        // 頁が被っていたら畳む。**畳まずに鍵だけ渡しても、
                        // 端末は頁の下で見えないまま**——指は届いても目が届かない。
                        self.right_focus = false;
                        self.focus_flash = Some(FocusFlash {
                            pane: FocusPane::Center,
                            ticks_remaining: FOCUS_FLASH_TICKS,
                        });
                        if self.browser_mode {
                            self.browser_mode = false;
                            return Task::batch([hide_browser(), self.terminal_focus()]);
                        }
                        return self.terminal_focus();
                    }
                    Shortcut::Center => {
                        self.focus_flash = Some(FocusFlash {
                            pane: FocusPane::Center,
                            ticks_remaining: FOCUS_FLASH_TICKS,
                        });
                        self.right_focus = false;
                        return if self.browser_mode {
                            focus_browser()
                        } else {
                            self.terminal_focus()
                        };
                    }
                    Shortcut::Right => {
                        // 頁を見ているときの ⌘L はアドレス欄——ブラウザの作法に
                        // 合わせる。頁が出ていなければ
                        // 今までどおり右の一覧へ鍵を渡す。
                        if self.browser_mode {
                            self.address_editing = true;
                            self.focus_flash = Some(FocusFlash {
                                pane: FocusPane::Center,
                                ticks_remaining: FOCUS_FLASH_TICKS,
                            });
                            // **順に流す。**並べて投げると、頁から鍵を外すより先に
                            // 欄へ渡してしまい、鍵は頁に残ったままになる。
                            let id = self.address_id.clone();
                            return blur_browser()
                                .chain(window_focus())
                                .chain(iced::widget::operation::focus(id.clone()))
                                // 打ち直しやすいように全部選んでおく——⌘L の
                                // 次の一打が今の URL を消してくれる。
                                .chain(iced::widget::operation::select_all(id));
                        }
                        // 鍵を受けるパネルへ(前に鍵を持っていたもの、無ければ右の列の
                        // いちばん上のもの)。
                        let Some(index) = self.keys_panel_to_enter() else {
                            return Task::none();
                        };
                        self.focus_flash = Some(FocusFlash {
                            pane: FocusPane::Right,
                            ticks_remaining: FOCUS_FLASH_TICKS,
                        });
                        return self.give_keys(index);
                    }
                    Shortcut::Close => {
                        if self.browser_mode {
                            return close_browser_tab();
                        }
                        self.tab_sync.invalidate();
                        terminal::close_tab();
                    }
                    Shortcut::Paste => return self.paste_into_terminal(),
                    Shortcut::ZoomIn => {
                        return self.apply_scale(font::stepped_scale(self.scale, 1));
                    }
                    Shortcut::ZoomOut => {
                        return self.apply_scale(font::stepped_scale(self.scale, -1));
                    }
                    Shortcut::ZoomReset => return self.apply_scale(font::DEFAULT_SCALE),
                    Shortcut::Help => return self.toggle_help(),
                    Shortcut::Appearance => return self.toggle_settings(),
                    // ▶ Restart の札と同じ道を通す——姿の控えも巻の控えも
                    // `Message::Restart` の側が面倒を見る。
                    Shortcut::Restart => return self.update(Message::Restart),
                }
                // タブの操作は Cockpit 自身が起こした変化なので、命令の完了直後に
                // 同じフレームで一覧へ写す。ここで使うのはタブ列挙(約20ms・定コスト)
                // だけ——ps や会話ログ読みはコストが変動するのでキー経路に置かない。
                // モデル名などの重い情報は名簿(約550ms)の後追いが埋める。
                self.right_focus = false;
                let tabs = self.request_tab_sync();
                let focus = self.terminal.as_ref().map_or_else(Task::none, |term| {
                    iced_term::TerminalView::focus(term.widget_id().clone())
                });
                Task::batch([tabs, focus])
            }
            Message::TabsSynced(generation, tabs) => {
                if !self.tab_sync.accept(generation, &tabs) {
                    // 切り替え前に取得を始めた結果は捨て、その場で取り直す。
                    return self.request_tab_sync();
                }
                self.apply_tabs(tabs);
                Task::none()
            }
            Message::TerminalPrepared(launch) => {
                self.terminal_preparing = false;
                Task::batch([self.attach_terminal(&launch), load_late_fonts()])
            }
            Message::LateFontsLoaded => Task::none(),
            Message::FieldPaste => {
                // 入力欄が自分で貼った。一覧の鍵受け(透明な欄)に鍵があるときだけ、
                // 今までどおり端末へ貼る。ほかの欄なら二重に入れない。
                if self.right_focus {
                    self.paste_into_terminal()
                } else {
                    Task::none()
                }
            }
            Message::KeySinkInput => {
                self.key_sink_text.clear();
                Task::none()
            }
            Message::TabStarted(result) => {
                if let Err(error) = result {
                    self.say(error);
                }
                self.request_tab_sync()
            }
            Message::PanelShortcut(letter) => match self.panel_with_shortcut(letter) {
                Some(index) => self.give_keys(index),
                None => Task::none(),
            },
            Message::PasteText(text) => {
                // 空の貼り付けは送らない。読めなかったときに改行だけ飛ぶと、
                // 打ちかけの行が勝手に確定する。
                if let Some(text) = text.filter(|text| !text.is_empty()) {
                    self.write_to_terminal(text.into_bytes());
                }
                Task::none()
            }
            Message::BrowserState(result) => match result {
                Ok(snapshot) => {
                    self.apply_browser_snapshot(snapshot);
                    if self.browser_tabs.is_empty() {
                        return self.terminal_focus();
                    }
                    Task::none()
                }
                Err(error) => {
                    self.browser_mode = false;
                    self.browser_error = Some(error);
                    Task::none()
                }
            },
            Message::WindowMeasured(size) => {
                // 窓が正本。控えのほうが古ければ、パネルの矩形ごと揃え直す。
                self.window_size = size;
                reconcile_browser(browser_bounds(size, self.scale))
            }
            Message::BrowserChanged(result) => match result {
                Ok(()) => Task::none(),
                Err(error) => {
                    self.browser_mode = false;
                    self.browser_error = Some(error);
                    hide_browser()
                }
            },
            Message::BrowserTranslation(reply) => deliver_translation(reply),
            Message::Restart => {
                // 全画面から押されたなら、新しい窓も全画面で立てる。窓のモードは非同期でしか読めないので、読んで
                // から焼く——ここで実体を起こしてしまうと、札を書く前に古い
                // 窓が消える。
                iced::window::latest().then(|id| match id {
                    Some(id) => iced::window::mode(id).map(Message::RestartWithMode),
                    None => Task::done(Message::RestartWithMode(
                        iced::window::Mode::Windowed,
                    )),
                })
            }
            Message::WindowPlaceNoted(mode, position) => {
                let fullscreen = mode == iced::window::Mode::Fullscreen;
                let previous = load_window_place();
                // **全画面のときの位置は当てにならない。** macOS はその space の
                // 原点を返すので、どの画面に居たかが 0,0 へ潰れる(実測2026-08-25)。
                // 窓の姿でいるときの位置だけを控え、全画面の回は前の値を守る。
                let position = if fullscreen {
                    previous.position
                } else {
                    position.or(previous.position)
                };
                save_window_place(WindowPlace {
                    fullscreen,
                    position,
                });
                Task::none()
            }
            Message::RestartWithMode(mode) => {
                if self.restart_pending {
                    return Task::none();
                }
                store_restart_note(&RestartNote {
                    fullscreen: mode == iced::window::Mode::Fullscreen,
                });
                // **次の窓が開くまで、この窓は落ちない。** `open` は Launch Services に
                // 頼んだ時点で返り、新しい窓が出るまで1秒近く間が空く。その間に
                // 古い窓が消えると前面に Cockpit が1枚も無くなり、macOS は Finder を
                // 前へ出す(全画面なら space ごと畳まれてデスクトップへ落ちる)。
                // 札を置いてから起こし、次の窓が `Opened` でそれを消すのを待つ。
                place_restart_handoff();
                match restart_current_executable() {
                    Ok(()) => {
                        self.restart_pending = true;
                        Task::perform(
                            async {
                                tokio::task::spawn_blocking(|| {
                                    wait_for_restart_handoff(
                                        &restart_handoff_path(),
                                        RESTART_HANDOFF_TIMEOUT,
                                    )
                                })
                                .await
                                .unwrap_or(false)
                            },
                            |_| Message::RestartHandoffDone,
                        )
                    }
                    Err(error) => {
                        clear_restart_handoff();
                        self.restart_error = Some(error);
                        Task::none()
                    }
                }
            }
            Message::RestartHandoffDone => self.exit_now(),
            Message::AddressChanged(value) => {
                self.address = value;
                self.address_editing = true;
                // 打った字は一覧の操作ではない(reddit.com の dd で完了させない)。
                self.right_focus = false;
                Task::none()
            }
            Message::AddressSubmitted => {
                let typed = self.address.trim().to_string();
                self.address_editing = false;
                self.right_focus = false;
                if typed.is_empty() {
                    return Task::none();
                }
                // 行き先へ移ったら鍵は頁へ返す。アドレス欄に居座らせない。
                Task::batch([navigate_browser(typed), focus_browser()])
            }
            Message::BrowserHistory(delta) => browser_history(delta),
            Message::BrowserReload => run_browser_effect(browser::reload_active),
            Message::BrowserOpenExternally => run_browser_effect(browser::open_active_externally),
            Message::SessionPressed(target) => {
                self.tab_sync.invalidate();
                if !terminal::select_tab(&target) {
                    return Task::none();
                }
                // 一覧は中央の表示モードと独立した航法面。ブラウザ・ヘルプ・
                // 設定のどこに居ても、選んだパネルを直ちに中央へ戻す。
                self.browser_mode = false;
                self.help_visible = false;
                self.settings_visible = false;
                self.help_restore_browser = false;
                self.right_focus = false;
                let tabs = self.request_tab_sync();
                Task::batch([tabs, hide_browser(), self.terminal_focus()])
            }
            Message::SessionsReordered => self.request_tab_sync(),
            Message::BrowserTabPressed(index) => {
                self.help_visible = false;
                self.settings_visible = false;
                self.help_restore_browser = false;
                self.dismiss_pending = true;
                self.browser_mode = true;
                self.right_focus = false;
                select_browser(index)
            }
            Message::NativeShortcutsReady => Task::none(),
            Message::FontsLoaded => {
                self.fonts_ready = true;
                self.open_terminal_if_ready()
            }
            Message::WindowEvent(event) => {
                match event {
                    iced::window::Event::Opened { size, .. } => {
                        // 前の窓への合図——この窓が実在した。待っている側はこれで落ちる。
                        clear_restart_handoff();
                        self.window_size = size;
                        self.window_opened = true;
                        let terminal_focus = self.open_terminal_if_ready();
                        let native_shortcuts = configure_native_shortcuts();
                        let fullscreen = self.restore_fullscreen_if_asked();
                        let mut tasks = vec![terminal_focus, native_shortcuts, fullscreen];
                        if !self.browser_revived {
                            self.browser_revived = true;
                            if !self.browser_tabs.is_empty() {
                                tasks.push(revive_browser(
                                    self.browser_tabs.clone(),
                                    self.browser_active,
                                    size,
                                    self.scale,
                                ));
                            }
                        }
                        // 初めて起こしたときだけ、はじめにの頁をブラウザで開く(前に出す)。
                        if let Some(url) = take_first_welcome() {
                            if self.shows_key("browser") {
                                self.browser_mode = true;
                                tasks.push(open_browser(browser::Source::Url(url), size, self.scale));
                            } else {
                                let _ = open_in_default_browser(&url);
                            }
                        }
                        return Task::batch(tasks);
                    }
                    iced::window::Event::Resized(size) => {
                        self.window_size = size;
                        let noted = self.note_window_place();
                        // 全画面へ移る間、窓の伸縮は何十回も来る。頁を1枚も
                        // 抱えていないときに WebView を主スレッドで叩き直す
                        // のは、その回数ぶん丸損になる。
                        if self.browser_tabs.is_empty() {
                            return noted;
                        }
                        return Task::batch([noted, resize_browser(size, self.scale)]);
                    }
                    // **窓が前へ戻ったら端末へ鍵を返す。** 端末のパネルは左押しでしか
                    // 鍵を受け取らない作りなので、他のアプリへ行って戻ると鍵が
                    // 誰にも無い状態になり、⏎ が呑まれる(音声入力のアプリの窓が一瞬
                    // 前に出る回もこれを踏む)。頁や欄を触っている最中は横取りしない。
                    iced::window::Event::Focused => {
                        if !self.browser_mode
                            && !self.help_visible
                            && !self.settings_visible
                            && !self.address_editing
                            && !self.right_focus
                        {
                            // **入力ソースは触らない。** IME の候補窓が開閉する
                            // だけでも `Focused` は飛ぶので、ここで切り替えると
                            // 変換中の文字が消える。
                            return self.terminal_focus_only();
                        }
                    }
                    iced::window::Event::CloseRequested => self.exit_now(),
                    _ => {}
                }
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // 頁を読んでいる間は、明滅と端末の起こしを間引く。ここは用事を捌くたびに通るので、印はここで立て直す。
        pacer::set_reading(self.browser_mode);
        iced_term::set_wakeup_interval(pacer::terminal_wakeup_interval());
        let mut subscriptions =
            vec![iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)];
        // ブラウザの伝言の見張り。**空のときは便を出さない。**`time::every(50ms)` で
        // 無条件に便を出すと、用事が無くても窓全体が秒に20回描き直される。
        // 見張り自体は裏の筋で 50ms ごとに続け、何か来ているときだけ便にする。
        if self.shows_key("browser") {
            subscriptions.push(iced::Subscription::run(browser_request_watch));
        }
        // パネルの点滅(focus_flash)は数コマだけ刻みが要る。その間だけ 50ms の時計を足す。
        if self.focus_flash.is_some() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(50)).map(|_| Message::BrowserPoll));
        }
        // 外から足したパネルの見張り。出している間だけ回す。
        for (index, panel) in self.custom.iter().enumerate() {
            let index = u16::try_from(index).unwrap_or(u16::MAX);
            if self.panels.shows(layout::PanelId(index)) {
                subscriptions.push(
                    panel
                        .subscription()
                        .with(index)
                        .map(|(index, payload)| Message::Custom(index, payload)),
                );
            }
        }
        subscriptions.push(iced::event::listen_with(shortcut_message));
        subscriptions.push(iced::event::listen_with(window_message));
        if let Some(term) = self.terminal.as_ref() {
            subscriptions.push(term.subscription().map(Message::Terminal));
        }
        Subscription::batch(subscriptions)
    }

    fn view(&self) -> Element<'_, Message> {
        let geo = geometry();
        let center_body: Element<'_, Message> = if self.settings_visible {
            settings::view().map(Message::Settings)
        } else if self.help_visible {
            help::view()
        } else if self.browser_mode {
            // WKWebViewがページを切り替える短い間にも、背後の端末を露出させない。
            container(self.browser_chrome())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_| palette::card_style())
                .into()
        } else if let Some(center) = self.panel_center() {
            center
        } else {
            self.terminal_panel().into()
        };
        let center_active = focus_flash_is_active(self.focus_flash, FocusPane::Center);
        // **縁は器の内側に描かれる。** 余白が縁(1px)より狭いと中身が縁の上に乗る。
        let center = container(center_body)
            .padding(1.0)
            .width(Length::FillPortion(geo.centre))
            .height(Length::Fill)
            .style(move |_| center_frame_style(center_active));
        // 左右の列は `panels.conf` の並びどおりに積む(`layout.rs`)。空の列は出さない。
        // ▶ Restart の札は右の列の下(右が空なら左の列の下)。
        // 列と列の間は線1本([`LINE`])。線が `geo.gap_x` の隙間そのものなので、
        // WebView の矩形([`browser_bounds`])と並びは同じ数から出る。
        let mut columns = row![].height(Length::Fill);
        if !self.panels.left.is_empty() {
            columns = columns
                .push(self.column_view(&self.panels.left, geo.left, self.panels.right.is_empty()))
                .push(seam(Axis::Vertical, geo.gap_x));
        }
        columns = columns.push(center);
        if !self.panels.right.is_empty() {
            columns = columns
                .push(seam(Axis::Vertical, geo.gap_x))
                .push(self.column_view(&self.panels.right, geo.right, true));
        }
        let window = container(columns)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: geo.pad_y,
            right: geo.pad_x,
            bottom: geo.pad_y,
            left: geo.pad_x,
        })
        .style(|_| window_style());
        // 鍵の預かり欄は窓の右下の隅に1画素で置く(`key_sink`)。
        stack![
            window,
            container(self.key_sink())
                .align_right(Length::Fill)
                .align_bottom(Length::Fill),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// タブ列挙を裏線で取りにいく。**主線で tmux を待たない**——tmux の応答は
    /// 起動直後(全パネルが一斉に描き直す間)で 0.5〜4 秒まで伸びた(2026-08-30 実測)。
    /// しかも主線が止まると端末の読み手も詰まり、tmux 側の書き出しが滞って
    /// 応答がさらに遅れる——待つ側が遅らせる輪になる。並行取得は1本に制限し、
    /// 取得中にタブが切り替わったら古い結果を捨てて取り直す。
    fn request_tab_sync(&mut self) -> Task<Message> {
        let Some(generation) = self.tab_sync.begin() else {
            return Task::none();
        };
        self.last_tab_sync = Instant::now();
        Task::perform(
            async { tokio::task::spawn_blocking(terminal::local_tabs).await.unwrap_or_default() },
            move |tabs| Message::TabsSynced(generation, tabs),
        )
    }

    fn apply_tabs(&mut self, tabs: Vec<armature_core::monitor::LocalTab>) {
        // 消えたタブへ閉じた刻を刻む。窓の外から閉じられた回(Stop フックの kill-pane・
        // claude 自身の終了)は `close_tab` を通らないので、ここで拾うしかない。
        // ⌘⇧T の時点でまとめて刻むと、何時間も前に死んだタブが「いま閉じた」扱いで
        // 最新になり、直前に閉じたタブより先に戻ってきた。
        session_ledger::sweep(&terminal::session_ids_of(&tabs));
        self.tabs.clone_from(&tabs);
    }


    /// 差し替えが済んだときだけ生える ▶ Restart の札。
    ///
    /// **反映の口だけは画面に残す。**押せなければ、新しい実体に入れ替われない。
    fn restart_badge(&self) -> Option<Element<'_, Message>> {
        if !self.restart_available {
            return None;
        }
        let badge = button(
            text(icon::RESTART)
                .size(11)
                .color(accent_active()),
        )
        .padding([2, 8])
        .on_press(Message::Restart)
        .style(|_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then_some(Background::Color(surface_raised())),
            text_color: text_muted(),
            border: Border {
                radius: palette::radius_control().into(),
                ..Border::default()
            },
            ..button::Style::default()
        });
        Some(
            // 左寄せ
            container(row![badge, Space::new().width(Length::Fill)])
                .padding([2, 4])
                .width(Length::Fill)
                .style(|_| {
                    let mut style = card_style();
                    style.shadow = iced::Shadow::default();
                    style
                })
                .into(),
        )
    }


    /// 前の窓が全画面だったなら、開いた窓をそのまま全画面へ入れる。
    ///
    /// 札は1度しか使わない——残すと、次に普通の窓で起こしたときまで全画面へ
    /// 引きずられる。
    /// 前の窓が全画面だったなら、開いた窓をそのまま全画面へ入れる。
    ///
    /// **窓が立ちきってから投げる。** 開いた直後に入れると呑まれて姿が変わらない。
    /// ひと呼吸置くぶん窓の姿が一瞬
    /// 見えるが、`set_mode` は `current_monitor` を見るので、控えた位置の画面で
    /// 全画面になる——窓をこしらえる時点の全画面では主画面に固定されてしまう。
    fn restore_fullscreen_if_asked(&mut self) -> Task<Message> {
        let asked = self.restore_fullscreen || load_window_place().fullscreen;
        self.restore_fullscreen = false;
        if !asked {
            return Task::none();
        }
        Task::future(async {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        })
        .then(|()| {
            iced::window::latest().then(|id| match id {
                Some(id) => iced::window::set_mode(id, iced::window::Mode::Fullscreen),
                None => Task::none(),
            })
        })
    }
    /// 窓がいまどんな姿でどこに居るかを控える。**次に起こすときの姿はここが正本。**
    ///
    /// 全画面へ出入りする間、伸縮は何十回も来る。答えは同じなので刻を置いて間引く。
    fn note_window_place(&mut self) -> Task<Message> {
        let now = Instant::now();
        if self
            .mode_noted_at
            .is_some_and(|at| now.duration_since(at) < MODE_NOTE_GAP)
        {
            return Task::none();
        }
        self.mode_noted_at = Some(now);
        iced::window::latest().then(|id| match id {
            Some(id) => iced::window::mode(id).then(move |mode| {
                iced::window::position(id)
                    .map(move |position| Message::WindowPlaceNoted(mode, position))
            }),
            None => Task::none(),
        })
    }

    fn terminal_panel(&self) -> iced::widget::Container<'_, Message> {
        let terminal: Element<'_, Message> = if let Some(term) = self.terminal.as_ref() {
            iced_term::TerminalView::show(term).map(Message::Terminal)
        } else if let Some(error) = self.terminal_error.as_deref() {
            container(text(error).color(accent_alert()))
                .center(Length::Fill)
                .into()
        } else {
            // まだ書体の到着待ち。すぐ端末が入るので静かに空けておく。
            Space::new().width(Length::Fill).height(Length::Fill).into()
        };

        container(terminal)
            // 下だけ詰める。最終行(auto mode の帯)と枠の間が空いて見えるのを
            // 利用者が嫌った(2026-08-19)。左右と上は今までどおり。
            .padding(iced::Padding {
                top: 10.0,
                right: 10.0,
                bottom: 2.0,
                left: 10.0,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| palette::terminal_seat_style())
    }



    /// パネル1枚に用事を渡し、そのパネルの頼みごと(`panel::Host`)を果たす。
    fn with_panel(
        &mut self,
        index: u16,
        run: impl FnOnce(&mut dyn panel::DynPanel, &mut panel::Host) -> Task<panel::Payload>,
    ) -> Task<Message> {
        let Some(panel) = self.custom.get_mut(usize::from(index)) else {
            return Task::none();
        };
        let mut host = panel::Host::default();
        let task = run(panel.as_mut(), &mut host).map(move |payload| Message::Custom(index, payload));
        let mut tasks = vec![task];
        for (request, reply) in host.requests {
            tasks.push(self.handle_panel_request(request, reply));
        }
        Task::batch(tasks)
    }

    /// 知らせを全部のパネルへ配る(出していないパネルにも——出したとき古い写しで描かないように)。
    fn tell_panels(&mut self, notice: &panel::Notice) -> Task<Message> {
        let count = u16::try_from(self.custom.len()).unwrap_or(u16::MAX);
        Task::batch((0..count).map(|index| {
            self.with_panel(index, |panel, host| panel.notice(notice, host))
        }).collect::<Vec<_>>())
    }

    /// 本体の状態のうちパネルが知りたいもの(端末のタブ・ブラウザのタブ)が前に配った写しと
    /// 違えば配り直す。用事を1つ捌くたびに通る。
    fn tell_changes(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        if self.told_tabs.as_ref() != Some(&self.tabs) {
            self.told_tabs = Some(self.tabs.clone());
            let notice = panel::Notice::Tabs(self.tabs.clone());
            tasks.push(self.tell_panels(&notice));
        }
        let browser = (
            self.browser_tabs
                .iter()
                .map(|tab| panel::BrowserTab {
                    title: tab.title.clone(),
                    url: tab.url.clone(),
                    asleep: tab.asleep,
                })
                .collect::<Vec<_>>(),
            self.browser_active,
            self.browser_mode,
        );
        if self.told_browser.as_ref() != Some(&browser) {
            self.told_browser = Some(browser.clone());
            let (tabs, active, showing) = browser;
            let notice = panel::Notice::BrowserTabs { tabs, active, showing };
            tasks.push(self.tell_panels(&notice));
        }
        if std::mem::take(&mut self.dismiss_pending) {
            tasks.push(self.dismiss_centers());
        }
        // 中央を借りたパネルが出たら、ヘルプ・設定・ブラウザの頁は退く(頁は窓の上に
        // 浮いていて、中央に描いたものを覆ってしまう)。
        if self.panel_center_open() {
            self.help_visible = false;
            self.settings_visible = false;
            if self.browser_mode {
                self.browser_mode = false;
                tasks.push(hide_browser());
            }
        }
        Task::batch(tasks)
    }

    /// 中央を借りているパネルの画面(先に並んでいるもの)。
    fn panel_center(&self) -> Option<Element<'_, Message>> {
        self.custom.iter().enumerate().find_map(|(index, panel)| {
            let id = u16::try_from(index).ok()?;
            if !self.shows_panel(index) {
                return None;
            }
            Some(panel.center()?.map(move |payload| Message::Custom(id, payload)))
        })
    }

    /// 中央を借りているパネルがあるか。
    fn panel_center_open(&self) -> bool {
        self.custom
            .iter()
            .enumerate()
            .any(|(index, panel)| self.shows_panel(index) && panel.center().is_some())
    }

    /// 中央を借りているパネルに閉じてもらう(Esc・ほかの画面が中央を取った)。
    fn dismiss_centers(&mut self) -> Task<Message> {
        let open: Vec<u16> = self
            .custom
            .iter()
            .enumerate()
            .filter(|(_, panel)| panel.center().is_some())
            .filter_map(|(index, _)| u16::try_from(index).ok())
            .collect();
        Task::batch(
            open.into_iter()
                .map(|index| self.with_panel(index, |panel, host| panel.notice(&panel::Notice::Dismiss, host)))
                .collect::<Vec<_>>(),
        )
    }

    /// その鍵のパネルが出ているか。本体が気にするのは `browser`(出ている間だけ頁を窓の中で
    /// 開く)だけ。
    fn shows_key(&self, key: &str) -> bool {
        layout::PanelId::from_key(key).is_some_and(|panel| self.panels.shows(panel))
    }

    fn shows_panel(&self, index: usize) -> bool {
        u16::try_from(index).is_ok_and(|index| self.panels.shows(layout::PanelId(index)))
    }

    /// 本体の短い知らせを出す(右の列の下に数秒)。
    fn say(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), Instant::now()));
    }

    /// ⌘ + `letter` を名乗った、出ているパネル。
    fn panel_with_shortcut(&self, letter: char) -> Option<u16> {
        (0..self.custom.len())
            .filter(|index| self.shows_panel(*index))
            .find(|index| {
                u16::try_from(*index)
                    .ok()
                    .and_then(|index| layout::PanelId(index).info())
                    .is_some_and(|info| info.shortcut == Some(letter))
            })
            .and_then(|index| u16::try_from(index).ok())
    }

    /// ⌘L で鍵を渡す先: 前に鍵を持っていたパネル、無ければ右の列(次に左の列)の上から
    /// 最初の、鍵を受けるパネル。
    fn keys_panel_to_enter(&self) -> Option<u16> {
        let takes = |panel: &layout::PanelId| panel.info().is_some_and(|info| info.keys);
        if let Some(index) = self.last_keys_panel
            && self.shows_panel(usize::from(index))
        {
            return Some(index);
        }
        self.panels
            .right
            .iter()
            .chain(&self.panels.left)
            .find(|panel| takes(panel))
            .and_then(|panel| match panel {
                layout::PanelId(index) => Some(*index),
            })
    }

    /// Claude に渡す約束の文を書き直す(出ているパネルの説明を上から順に)。
    fn write_notes(&self) {
        let notes: Vec<String> = self
            .panels
            .right
            .iter()
            .chain(&self.panels.left)
            .filter_map(|panel| match panel {
                layout::PanelId(index) => self.custom.get(usize::from(*index)),
            })
            .filter_map(|panel| panel.note())
            .collect();
        notes::write(&notes);
    }

    /// パネルに鍵を渡す。鍵を受けないパネルなら何もしない。
    fn give_keys(&mut self, index: u16) -> Task<Message> {
        let takes = layout::PanelId(index)
            .info()
            .is_some_and(|info| info.keys);
        if !takes {
            return Task::none();
        }
        let mut tasks = Vec::new();
        if self.keys_panel != Some(index) {
            tasks.push(self.drop_keys());
            self.keys_panel = Some(index);
            self.last_keys_panel = Some(index);
            tasks.push(self.with_panel(index, |panel, host| panel.notice(&panel::Notice::Keys(true), host)));
        }
        self.right_focus = true;
        input_source::to_ascii();
        tasks.push(blur_browser());
        tasks.push(iced::widget::operation::focus(self.right_focus_id.clone()));
        Task::batch(tasks)
    }

    /// 鍵を持っているパネルから鍵を下ろす(知らせだけ。端末へ渡すのは呼ぶ側)。
    fn drop_keys(&mut self) -> Task<Message> {
        self.right_focus = false;
        match self.keys_panel.take() {
            Some(index) => {
                self.with_panel(index, |panel, host| panel.notice(&panel::Notice::Keys(false), host))
            }
            None => Task::none(),
        }
    }

    /// 外から足したパネルの頼みごと(`panel::Host`)を果たし、結果を `reply` で返す
    /// (パネルが `Outcome::map` で聞いていれば届く。聞いていなければ捨てられる)。
    fn handle_panel_request(
        &mut self,
        request: panel::Request,
        reply: panel::Reply,
    ) -> Task<Message> {
        match request {
            panel::Request::OpenUrl(url) => {
                // 通すのは web の頁だけ。file: や独自の scheme(手元のアプリを起こす)は断る。
                let Some(url) = browser::checked_web_url(&url) else {
                    let _ = reply.send(Err(tr!(
                        format!("Not a web page: {url}"),
                        format!("web の頁ではない: {url}"),
                    )
                    .to_string()));
                    return Task::none();
                };
                if !self.shows_key("browser") {
                    let _ = reply.send(open_in_default_browser(&url));
                    return Task::none();
                }
                let _ = reply.send(Ok(()));
                self.dismiss_pending = true;
                self.settings_visible = false;
                self.help_visible = false;
                self.right_focus = false;
                self.browser_error = None;
                self.browser_mode = true;
                open_browser(browser::Source::Url(url), self.window_size, self.scale)
            }
            panel::Request::SelectTab(target) => {
                let _ = reply.send(Ok(()));
                self.update(Message::SessionPressed(target))
            }
            panel::Request::MoveTabs(order) => Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || terminal::reorder_tabs(&order)).await
                },
                move |_| {
                    let _ = reply.send(Ok(()));
                    Message::SessionsReordered
                },
            ),
            panel::Request::SelectBrowserTab(index) => {
                let _ = reply.send(Ok(()));
                self.update(Message::BrowserTabPressed(index))
            }
            panel::Request::ReleaseKeys => {
                let _ = reply.send(Ok(()));
                let drop = self.drop_keys();
                Task::batch([drop, self.terminal_focus()])
            }
            panel::Request::StartClaude { title, arguments } => {
                self.tab_sync.invalidate();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || terminal::start_claude(&title, &arguments))
                            .await
                            .map_err(|error| format!("claude launcher thread ended: {error}"))?
                    },
                    move |result: Result<String, String>| {
                        let _ = reply.send(result.clone().map(drop));
                        Message::TabStarted(result)
                    },
                )
            }
        }
    }

    /// 並びが変わった窓の後始末。外したパネルの鍵を手放し、出したパネルの中身を取りに行く。
    /// ブラウザの頁は窓の上に別に浮いているので、置き場を測り直す。
    fn after_panels_changed(&mut self) -> Task<Message> {
        let mut tasks = vec![resize_browser(self.window_size, self.scale)];
        if !self.shows_key("browser") && self.browser_mode {
            self.browser_mode = false;
            tasks.push(hide_browser());
            tasks.push(self.terminal_focus());
        }
        self.write_notes();
        let keys_gone = self
            .keys_panel
            .is_some_and(|index| !self.shows_panel(usize::from(index)));
        if keys_gone && self.right_focus {
            self.right_focus = false;
            tasks.push(self.terminal_focus());
        }
        Task::batch(tasks)
    }

    /// 左右の列の1本。`panels` を上から順に積む。列の残りの高さは、埋めるパネル
    /// (ブラウザのタブ・タスク)が分け合う。
    fn column_view<'a>(
        &'a self,
        panels: &'a [layout::PanelId],
        portion: u16,
        with_restart: bool,
    ) -> Element<'a, Message> {
        let gap = geometry().gap_y;
        let mut column = column![]
            .width(Length::FillPortion(portion))
            .height(Length::Fill);
        for (index, panel) in panels.iter().enumerate() {
            // パネルとパネルの間は線1本。**時計のすぐ下の暦だけは線を引かない**
            // ——時刻と日付は1枚の画面として読むもので、間に線があると2つの計器に割れる。
            if index > 0 && !joined(panels[index - 1], *panel) {
                column = column.push(seam(Axis::Horizontal, gap));
            }
            column = column.push(self.panel_view(*panel));
        }
        // 埋めるパネルが無い列は、下の空きを画面の地のまま伸ばす。
        if !panels.iter().any(|panel| panel.fills()) {
            column = column.push(Space::new().width(Length::Fill).height(Length::Fill));
        }
        if with_restart && let Some(restart) = self.restart_badge() {
            column = column.push(seam(Axis::Horizontal, gap)).push(restart);
        }
        column.into()
    }

    /// パネル1枚の描き方。中身はパネルが描き(`Panel::view`)、面と余白はこちらが着せる。
    /// 鍵を受けるパネル(`Panel::KEYS`)は、どこを押しても鍵が渡るように包む。
    fn panel_view(&self, panel: layout::PanelId) -> Element<'_, Message> {
        let layout::PanelId(index) = panel;
        let body: Element<'_, Message> = match self.custom.get(usize::from(index)) {
            Some(view) => view
                .view()
                .map(move |payload| Message::CustomInput(index, payload)),
            None => Space::new().into(),
        };
        let body = container(body)
            .padding(if panel.padded() {
                iced::Padding::from([10, 12])
            } else {
                iced::Padding::ZERO
            })
            .width(Length::Fill)
            .style(|_| palette::card_style());
        let body: Element<'_, Message> = if panel.fills() {
            body.height(Length::Fill).into()
        } else {
            body.into()
        };
        if panel.info().is_some_and(|info| info.keys) {
            tap::tap(body, move |_, _| Message::PanelPressed(index))
        } else {
            body
        }
    }


    /// 鍵を受けるパネルが鍵を持っている間、焦点を預かる見えない欄。端末から焦点を外して
    /// おくためだけにあり、打たれた字は捨てる(打鍵はパネルへ `Panel::key` で届く)。
    fn key_sink(&self) -> Element<'_, Message> {
        text_input("", &self.key_sink_text)
            .id(self.right_focus_id.clone())
            .on_input(|_| Message::KeySinkInput)
            .padding(0)
            .size(1)
            .width(Length::Fixed(1.0))
            .style(|_, _| text_input::Style {
                background: Background::Color(iced::Color::TRANSPARENT),
                border: Border::default(),
                icon: iced::Color::TRANSPARENT,
                placeholder: iced::Color::TRANSPARENT,
                value: iced::Color::TRANSPARENT,
                selection: iced::Color::TRANSPARENT,
            })
            .into()
    }

    /// cmd+V を窓側で受ける。
    ///
    /// 画像なら端末へ Ctrl+V(0x16)を送る。Claude Code はそれを受けて自分で
    /// クリップボードから画像を取り込むので、利用者が Ctrl+V を押したときと同じ道
    /// になる。**字は窓側で読んで PTY へ流す。**
    ///
    /// 以前は字の貼り付けを端末(iced_term)へ任せていた。だがあのパネルは
    /// **指の下にいるときしか鍵を受け取らない**——`handle_focus` が左押しの位置
    /// だけで焦点を決めるので、暦や一覧を一度押した時点で端末は鍵を失い、以後
    /// ⌘V が黙る。
    ///
    /// 2026-08-17 に一度「横取りして壊した」のは、横取りしたのに字を書かずに
    /// 帰っていたため。いまは窓側が最後まで面倒を見るので、端末側の ⌘V の割当は
    /// 外してある(`vendor/iced_term/src/bindings.rs`)——二重に貼らないため。
    fn paste_into_terminal(&mut self) -> Task<Message> {
        if self.browser_mode || self.help_visible || self.settings_visible {
            return Task::none();
        }
        let has_image = apple_translation::clipboard_has_image();
        if has_image {
            self.write_to_terminal(vec![CTRL_V]);
            return Task::none();
        }
        // **その場で読む。** `iced::clipboard::read` は非同期で、答えが返るのは
        // 次の周回になる。音声入力(音声入力のアプリ)は ⌘V を送った直後に元の
        // クリップボードへ戻すので、周回を1つ待つと**戻されたあとの中身**を
        // 読んでしまい、何も貼られない。
        if let Some(text) = apple_translation::clipboard_text() {
            self.write_to_terminal(text.into_bytes());
            // 貼った先へ鍵も返す。字は PTY へ直に書くので鍵が無くても入るが、
            // **その直後の ⏎ は鍵が要る**——ここを揃えないと「貼れたのに送れない」。
            // 入力ソースは触らない(変換中の文字を捨てないため)。
            return self.terminal_focus_only();
        }
        // 橋が字を返さなかったときだけ、今までどおり非同期の口へ落とす
        // ——画像でも字でもない何か(ファイルの約束等)は iced 側が拾えることがある。
        iced::clipboard::read().map(Message::PasteText)
    }

    fn write_to_terminal(&mut self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        if let Some(term) = self.terminal.as_mut() {
            let _ = term.handle(iced_term::Command::ProxyToBackend(
                iced_term::BackendCommand::Write(bytes),
            ));
        }
    }


    /// 窓が開き、書体が font system へ届いてから端末を起こす。
    ///
    /// iced_term はセル寸法を iced のテキスト測定で得るので、書体が届く前に
    /// 端末を作ると代替書体の幅で桁数が決まる。中の tmux と TUI はその桁数で
    /// 画面を組み、一度吐かれた出力は後から桁数が直っても組み直されない。
    /// ライブラリの前提(登録済みで作る)を満たす側に合わせる。
    fn open_terminal_if_ready(&mut self) -> Task<Message> {
        if self.terminal.is_some()
            || self.terminal_preparing
            || !self.window_opened
            || !self.fonts_ready
        {
            return Task::none();
        }
        // tmux の準備(has-session・set-option の連打)は裏線で。主線で待つと
        // 起動直後に 1.9 秒止まった(2026-08-30 実測)。
        self.terminal_preparing = true;
        Task::perform(
            async { tokio::task::spawn_blocking(terminal::prepare).await.ok() },
            |launch| match launch {
                Some(launch) => Message::TerminalPrepared(launch),
                None => Message::Tick,
            },
        )
    }

    /// 準備の整った起動口で端末を作り、中央へ鍵を入れる。
    fn attach_terminal(&mut self, launch: &terminal::Launch) -> Task<Message> {
        if self.terminal.is_some() {
            return Task::none();
        }
        match iced_term::Terminal::new(0, terminal::settings(launch)) {
            Ok(terminal) => {
                self.terminal = Some(terminal);
            }
            Err(error) => {
                self.terminal_error = Some(tr!(
                    format!("Couldn't start the terminal: {error}"),
                    format!("端末を初期化できなかった: {error}"),
                ));
                return Task::none();
            }
        }
        let Some(terminal) = self.terminal.as_mut() else {
            return Task::none();
        };
        terminal::suppress_app_shortcuts(terminal);
        iced_term::TerminalView::focus(terminal.widget_id().clone())
    }


    /// 開いている資料が書き替わっていたら、同じタブへ最新版を載せ直す。
    ///
    /// 見えていないときは触らない——裏で差し替えても読めず、次に出したときに
    /// 読んでいた位置だけが飛ぶ。刻が読めない回(書き替えの最中)は次の周期に回す。
    fn reload_watched_file(&mut self) {
        if !self.browser_mode {
            return;
        }
        let Some(watched) = self.watched_file.as_mut() else {
            return;
        };
        let Some(stamp) = file_stamp(&watched.path) else {
            return;
        };
        if stamp == watched.stamp {
            return;
        }
        watched.stamp = stamp;
        let source = browser::source_for_path(&watched.path.clone());
        if let Err(error) = browser::reload_source(&source) {
            self.browser_error = Some(error);
        }
    }

    /// 中央の会話端末へ戻り、右パネルへ移る前の入力ソースを復元する。
    fn terminal_focus(&self) -> Task<Message> {
        input_source::restore();
        self.terminal_focus_only()
    }

    /// 端末へ鍵だけ返す。**入力ソースには触らない。**
    ///
    /// `terminal_focus` は右パネルで英字へ倒した入力ソースを戻す。あれを
    /// 打鍵の最中に走らせると、**変換中の文字が捨てられる**——入力ソースの
    /// 切り替えは IME の未確定文字を破棄する。窓が前へ戻るたびに呼ぶような口は、こちらを使う。
    fn terminal_focus_only(&self) -> Task<Message> {
        self.terminal.as_ref().map_or_else(Task::none, |term| {
            iced_term::TerminalView::focus(term.widget_id().clone())
        })
    }

    fn exit_now(&mut self) -> ! {
        // 外から足したパネルに終わりを知らせる(`Panel::on_exit`)。このあと Drop は走らない。
        for panel in &mut self.custom {
            panel.on_exit();
        }
        // Wry の子 WKWebView は通常の Drop で数秒待つ場合がある。窓を閉じる
        // 操作では tmux を残したままプロセスだけを確実に終える。
        std::process::exit(0)
    }

    fn open_new_browser_tab(&mut self) -> Task<Message> {
        self.help_visible = false;
        self.settings_visible = false;
        self.help_restore_browser = false;
        self.dismiss_pending = true;
        self.browser_mode = true;
        self.right_focus = false;
        self.browser_error = None;
        // 新しいタブは行き先が空。頁の中に欄を持たせず、アドレス欄へ鍵を渡して
        // すぐ打てる状態にする。
        self.address.clear();
        self.address_editing = true;
        let id = self.address_id.clone();
        open_browser(
            browser::new_tab_source(&self.browser_tabs),
            self.window_size,
            self.scale,
        )
        .chain(blur_browser())
        .chain(window_focus())
        .chain(iced::widget::operation::focus(id))
    }

    fn toggle_help(&mut self) -> Task<Message> {
        if self.help_visible {
            self.help_visible = false;
            self.settings_visible = false;
            let restore = self.help_restore_browser && !self.browser_tabs.is_empty();
            self.help_restore_browser = false;
            if restore {
                self.dismiss_pending = true;
                self.browser_mode = true;
                return show_browser(self.window_size, self.scale);
            }
            return self.terminal_focus();
        }

        self.help_visible = true;
        self.settings_visible = false;
        self.help_restore_browser = self.browser_mode;
        self.right_focus = false;
        if self.browser_mode {
            return hide_browser();
        }
        Task::none()
    }

    /// 設定を開閉する。
    ///
    /// ヘルプとまったく同じ作法——中央のパネルを借りるので、ブラウザが出ていたら
    /// 退かせ、閉じるときに戻す。**両方を同時に出さない**(どちらも中央1枚を
    /// 占めるので、重ねると下が死ぬ)。
    fn toggle_settings(&mut self) -> Task<Message> {
        if self.settings_visible {
            self.settings_visible = false;
            let restore = self.settings_restore_browser && !self.browser_tabs.is_empty();
            self.settings_restore_browser = false;
            if restore {
                self.dismiss_pending = true;
                self.browser_mode = true;
                return show_browser(self.window_size, self.scale);
            }
            return self.terminal_focus();
        }

        self.settings_visible = true;
        self.help_visible = false;
        self.settings_restore_browser = self.browser_mode;
        self.right_focus = false;
        if self.browser_mode {
            return hide_browser();
        }
        Task::none()
    }

    fn apply_browser_snapshot(&mut self, snapshot: browser::Snapshot) {
        browser::save(&snapshot);
        self.browser_tabs = snapshot.tabs;
        self.browser_active = snapshot.active;
        self.browser_mode = snapshot.visible;
        self.browser_error = None;
        self.sync_address();
    }




    /// アドレス欄をいま見ている頁へ合わせる。
    ///
    /// 打っている最中は触らない——入力の途中で字が入れ替わるのが一番困る。
    fn sync_address(&mut self) {
        if self.address_editing {
            return;
        }
        self.address = self
            .browser_tabs
            .get(self.browser_active)
            .map(|tab| tab.url.clone())
            .unwrap_or_default();
    }

    /// 頁の上に渡す帯。戻る・進む・読み直し・アドレス欄・外のブラウザで開く。
    ///
    /// WebView は Iced の外に浮いているので、この帯の高さぶんだけ
    /// [`browser_bounds`] が頁を下げている。片方だけ変えると頁が帯へ潜る。
    fn browser_chrome(&self) -> Element<'_, Message> {
        let step = |label: Element<'static, Message>, message: Message| {
            button(label)
                .padding([2, 7])
                .on_press(message)
                .style(|_, status| button::Style {
                    background: matches!(
                        status,
                        button::Status::Hovered | button::Status::Pressed
                    )
                    .then_some(Background::Color(surface_raised())),
                    text_color: text_muted(),
                    border: Border {
                        radius: palette::radius_control().into(),
                        ..Border::default()
                    },
                    ..button::Style::default()
                })
        };
        let address = text_input("", &self.address)
            .id(self.address_id.clone())
            .on_input(Message::AddressChanged)
            .on_submit(Message::AddressSubmitted)
            .padding([4, 10])
            .size(12)
            .width(Length::Fill)
            .style(|_, status| {
                let focused = matches!(status, text_input::Status::Focused { .. });
                text_input::Style {
                    background: Background::Color(surface_window()),
                    border: Border {
                        color: if focused {
                            iced::Color {
                                a: 0.55,
                                ..accent_focus()
                            }
                        } else {
                            palette::surface_edge()
                        },
                        width: 1.0,
                        radius: palette::radius_control().into(),
                    },
                    icon: text_muted(),
                    placeholder: text_faint(),
                    value: text_primary(),
                    selection: iced::Color {
                        a: 0.35,
                        ..accent_focus()
                    },
                }
            });
        let bar = row![
            step(chrome_glyph("‹"), Message::BrowserHistory(-1)),
            step(chrome_glyph("›"), Message::BrowserHistory(1)),
            step(chrome_glyph("⟳"), Message::BrowserReload),
            address,
            // ☆(栞)はここから外した——栞は ⌘D で入れる。
            step(external_mark(), Message::BrowserOpenExternally),
        ]
        .spacing(4)
        .align_y(Alignment::Center);

        column![
            container(bar)
                .padding([5, 8])
                .width(Length::Fill)
                .height(Length::Fixed(BROWSER_CHROME_HEIGHT as f32)),
            // 帯の下は頁の場所。WebView が自分で浮くので、ここは空けておく。
            Space::new().height(Length::Fill),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn handle_browser_request(&mut self, request: browser::Request) -> Task<Message> {
        match request {
            browser::Request::Hide => {
                self.browser_mode = false;
                self.right_focus = false;
                Task::batch([hide_browser(), self.terminal_focus()])
            }
            browser::Request::FocusAddress => {
                self.address_editing = true;
                self.right_focus = false;
                self.focus_flash = Some(FocusFlash {
                    pane: FocusPane::Center,
                    ticks_remaining: FOCUS_FLASH_TICKS,
                });
                let id = self.address_id.clone();
                blur_browser()
                    .chain(window_focus())
                    .chain(iced::widget::operation::focus(id.clone()))
                    .chain(iced::widget::operation::select_all(id))
            }
            browser::Request::Reload => run_browser_effect(browser::reload_active),
            browser::Request::FocusCenter => {
                self.focus_flash = Some(FocusFlash {
                    pane: FocusPane::Center,
                    ticks_remaining: FOCUS_FLASH_TICKS,
                });
                self.right_focus = false;
                focus_browser()
            }
            browser::Request::CloseTab => close_browser_tab(),
            browser::Request::NextTab => cycle_browser(1),
            browser::Request::PreviousTab => cycle_browser(-1),
            browser::Request::NewTab(url) => {
                self.dismiss_pending = true;
                self.browser_mode = true;
                open_browser(browser::source_for_url(&url), self.window_size, self.scale)
            }
            // 頁の中のリンク。**同じタブへ載せ替える**ので履歴が溜まり、‹ が効く。
            browser::Request::NavigateActive(url) => {
                self.dismiss_pending = true;
                self.browser_mode = true;
                navigate_browser(url)
            }
            browser::Request::NewBlank => self.open_new_browser_tab(),
            browser::Request::NewShell => self.update(Message::Shortcut(Shortcut::Shell)),
            browser::Request::Reopen => self.update(Message::Shortcut(Shortcut::Reopen)),
            browser::Request::Settings => self.toggle_settings(),
            browser::Request::Help => self.toggle_help(),
            browser::Request::Bookmark => run_browser_effect(browser::bookmark_active),
            browser::Request::History(delta) => browser_history(delta),
            browser::Request::Restart => self.update(Message::Restart),
            browser::Request::FocusTerminal => self.update(Message::Shortcut(Shortcut::Terminal)),
            browser::Request::OpenBookmark { id, nonce, url } => {
                open_browser_bookmark(id, nonce, url)
            }
            browser::Request::DeleteBookmark { id, nonce, spec } => {
                delete_browser_bookmark(id, nonce, spec)
            }
            browser::Request::SelectTab(index) => select_browser(index),
            browser::Request::Title { id, title } => update_browser_metadata(id, Some(title), None),
            browser::Request::Location { id, url } => update_browser_metadata(id, None, Some(url)),
            browser::Request::Translate {
                id,
                nonce,
                ask,
                claude,
                texts,
            } => translate_browser(id, nonce, ask, claude, texts),
        }
    }
}


/// ブラウザの帯の字の印(‹ › ⟳)。大きさと色は外部ボタンの線画と揃える。
const CHROME_GLYPH_SIZE: f32 = 13.0;

fn chrome_glyph(label: &'static str) -> Element<'static, Message> {
    text(label).size(CHROME_GLYPH_SIZE).color(text_muted()).into()
}

/// 帯の「外のブラウザで開く」。Lucide の external-link(ISC)の線画。
///
/// 書体の字(Nerd Font の \u{f08e})で出したら、面いっぱいに塗った太い印に
/// なって隣の ‹ › ⟳ から浮いた。
/// 線画を字の丈の中に小さく置き、色も字と同じ `text_muted` にする。
fn external_mark() -> Element<'static, Message> {
    // icons: Lucide (ISC) https://lucide.dev
    let bytes: &'static [u8] = include_bytes!("../assets/icons/external-link.svg");
    let line = (CHROME_GLYPH_SIZE * 1.3).round();
    container(
        iced::widget::svg(iced::widget::svg::Handle::from_memory(bytes))
            .width(Length::Fixed(11.0))
            .height(Length::Fixed(11.0))
            .style(|_, _| iced::widget::svg::Style {
                color: Some(text_muted()),
            }),
    )
    .height(Length::Fixed(line))
    .center_y(Length::Fixed(line))
    .into()
}


/// ブラウザで開いている資料と、載せた時点の刻。
struct WatchedFile {
    path: PathBuf,
    stamp: (std::time::SystemTime, u64),
}

/// 更新時刻と大きさ。どちらかが動いていれば中身が変わったとみなす。
fn file_stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// 窓の出来事のうち、盤が実際に使う4つだけを拾う。
///
/// `iced::window::events()` をそのまま繋ぐと `RedrawRequested` まで流れてくる。
/// あれは「描いた」の報せなので、メッセージにすると
/// 描く → 報せ → メッセージ → update と view の作り直し → 描き直しの要求 → 描く
/// という輪ができ、何も起きていなくても窓が表示更新率で回り続ける。
/// 2026-08-18 の実測では、待機中の実体が常時 58% の時間を描画観測子の中で過ごし、
/// うち 42% は次の描画面を待って止まっていた(端末の字は毎フレーム 3,400〜4,300 回
/// 個別に描いている)。ここで濾して、輪を切る。
fn window_message(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    let iced::Event::Window(event) = event else {
        return None;
    };
    matches!(
        event,
        iced::window::Event::Opened { .. }
            | iced::window::Event::Resized(_)
            | iced::window::Event::Focused
            | iced::window::Event::Unfocused
            | iced::window::Event::CloseRequested
    )
    .then(|| Message::WindowEvent(event))
}

fn shortcut_message(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
        key,
        physical_key,
        modifiers,
        ..
    }) = event
    else {
        return None;
    };
    let picked = pick_shortcut(&key, physical_key, modifiers);
    // ⌘V を入力欄が貼ったなら、端末へ流すかは器が焦点を見て決める。
    if matches!(picked, Some(Message::Shortcut(Shortcut::Paste)))
        && status == iced::event::Status::Captured
    {
        return Some(Message::FieldPaste);
    }
    picked
}

/// 打鍵の行き先。Armature 自身の ⌘ キー → パネルが名乗った ⌘ + 字 → 鍵を持っているパネル。
fn pick_shortcut(
    key: &Key,
    physical_key: iced::keyboard::key::Physical,
    modifiers: Modifiers,
) -> Option<Message> {
    if let Some(shortcut) = shortcut_for_key(key, physical_key, modifiers) {
        return Some(Message::Shortcut(shortcut));
    }
    if modifiers == Modifiers::COMMAND
        && let Some(letter) = key.to_latin(physical_key)
        && layout::PanelId::all()
            .into_iter()
            .any(|panel| panel.info().is_some_and(|info| info.shortcut == Some(letter)))
    {
        return Some(Message::PanelShortcut(letter));
    }
    Some(Message::PanelKey(panel::KeyPress {
        key: key.clone(),
        physical: physical_key,
        modifiers,
    }))
}

fn terminal_command_claims_focus(command: &iced_term::BackendCommand) -> bool {
    matches!(
        command,
        iced_term::BackendCommand::SelectStart(..)
            | iced_term::BackendCommand::SelectUpdate(..)
            | iced_term::BackendCommand::MouseReport(..)
    )
}






/// ⌘ , で設定。macOS の「設定」の定位置に合わせている。
///
/// **拾い方を3通り用意する。** `to_latin` は「1文字で U+370 未満なら素通し、
/// でなければ物理キーの表を引く」作りだが、その表は英字と数字しか持たない
/// ([`iced::keyboard::key::Key::to_latin`])。日本語入力が有効だと論理キーが
/// 読点(U+3001)になり得て、そのとき表に無い読点は `None` へ落ちる。
/// 論理キー・読点・物理キーの順に見て、どれか当たれば開く。
fn appearance_for_key(
    key: &Key,
    physical_key: iced::keyboard::key::Physical,
    modifiers: Modifiers,
) -> bool {
    if modifiers != Modifiers::COMMAND {
        return false;
    }
    if key.to_latin(physical_key) == Some(',') {
        return true;
    }
    if matches!(key, Key::Character(value) if value == "、" || value == ",") {
        return true;
    }
    matches!(
        physical_key,
        iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::Comma)
    )
}

/// ⌘+ で拡大・⌘- で縮小・⌘0 で既定へ。ブラウザや端末と同じ作法。
///
/// `+` は JIS でも US でも Shift を伴うので ⌘⇧ も受ける。US 配列で Shift を
/// 押さずに刻めるよう `=` も拡大に充てる。
fn zoom_for_key(
    key: &Key,
    physical_key: iced::keyboard::key::Physical,
    modifiers: Modifiers,
) -> Option<Shortcut> {
    if modifiers != Modifiers::COMMAND && modifiers != Modifiers::COMMAND | Modifiers::SHIFT {
        return None;
    }
    match key.to_latin(physical_key)? {
        '+' | '=' => Some(Shortcut::ZoomIn),
        '-' | '_' => Some(Shortcut::ZoomOut),
        '0' => Some(Shortcut::ZoomReset),
        _ => None,
    }
}

fn shortcut_for_key(
    key: &Key,
    physical_key: iced::keyboard::key::Physical,
    modifiers: Modifiers,
) -> Option<Shortcut> {
    if key.to_latin(physical_key) == Some('/') && modifiers == Modifiers::COMMAND {
        return Some(Shortcut::Help);
    }
    if appearance_for_key(key, physical_key, modifiers) {
        return Some(Shortcut::Appearance);
    }
    if key.to_latin(physical_key) == Some('t') {
        return match modifiers {
            Modifiers::COMMAND => Some(Shortcut::Claude),
            // 閉じたタブを戻す。ブラウザの ⇧⌘T と同じ作法に合わせている。
            value if value == Modifiers::COMMAND | Modifiers::SHIFT => Some(Shortcut::Reopen),
            _ => None,
        };
    }
    if key.to_latin(physical_key) == Some('j') && modifiers == Modifiers::COMMAND {
        return Some(Shortcut::Shell);
    }
    if key.to_latin(physical_key) == Some('w') && modifiers == Modifiers::COMMAND {
        return Some(Shortcut::Close);
    }
    if key.to_latin(physical_key) == Some('r') {
        return match modifiers {
            // 窓ごと起こし直す。**shift を要る**ようにしてあるのは、⌘R を
            // 打ち損じただけで作業中の窓が飛ぶのを避けるため。
            value if value == Modifiers::COMMAND | Modifiers::SHIFT => Some(Shortcut::Restart),
            _ => None,
        };
    }
    if key.to_latin(physical_key) == Some('b') && modifiers == Modifiers::COMMAND {
        return Some(Shortcut::Browser);
    }
    if key.to_latin(physical_key) == Some('h') && modifiers == Modifiers::COMMAND {
        return Some(Shortcut::Center);
    }
    if key.to_latin(physical_key) == Some('l') && modifiers == Modifiers::COMMAND {
        return Some(Shortcut::Right);
    }
    if key.to_latin(physical_key) == Some('v') && modifiers == Modifiers::COMMAND {
        return Some(Shortcut::Paste);
    }
    if let Some(zoom) = zoom_for_key(key, physical_key, modifiers) {
        return Some(zoom);
    }
    // ⌘⏎ は中央の端末へ。**⌘H と別の鍵にしてある**——⌘H は「中央のパネル」で、
    // 頁が出ていれば頁へ渡す。こちらは頁を畳んででも端末へ渡す。⏎ を選んだのは「入力欄へ行く」の指が既に知っているから。
    if key == &Key::Named(Named::Enter) && modifiers == Modifiers::COMMAND {
        return Some(Shortcut::Terminal);
    }
    if key == &Key::Named(Named::Tab) {
        return match modifiers {
            Modifiers::CTRL => Some(Shortcut::Next),
            value if value == Modifiers::CTRL | Modifiers::SHIFT => Some(Shortcut::Previous),
            _ => None,
        };
    }
    None
}


/// 線の向き。
#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

/// パネルの境の線。**境はこの1本だけ**——パネルの面は地と同じ画面の色なので、
/// 線が無いと隣のパネルと溶ける。
fn seam<'a>(axis: Axis, width: f32) -> Element<'a, Message> {
    let style = |_: &iced::Theme| rule::Style {
        color: palette::surface_line(),
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    };
    match axis {
        Axis::Horizontal => rule::horizontal(width).style(style).into(),
        Axis::Vertical => rule::vertical(width).style(style).into(),
    }
}

/// 上下に並んだ2枚の間に線を引かないか。時計の下の暦だけ。
fn joined(above: layout::PanelId, below: layout::PanelId) -> bool {
    joined_keys(above.key(), below.key())
}

fn joined_keys(above: &str, below: &str) -> bool {
    above == "clock" && below == "calendar"
}

/// パネルの面。単色の面と角丸だけ。
fn panel_style(background: iced::Color, radius: f32) -> container::Style {
    container::Style {
        text_color: Some(text_primary()),
        background: Some(Background::Color(background)),
        border: Border {
            radius: radius.into(),
            ..Border::default()
        },
        ..Default::default()
    }
}

/// いちばん外側の地。**縁を引かない**。
fn window_style() -> container::Style {
    panel_style(surface_window(), 0.0)
}

/// パネルのカード。地(`surface_window`)から1段浮いた面だけで境界を出し、枠線は引かない
/// ——面が同じ高さのまま線だけで仕切ると、パネルの数だけ明るい線が走って画面が騒がしくなる。
fn card_style() -> container::Style {
    panel_style(surface_card(), palette::radius_card())
}


/// 中央の枠。フォーカスが当たっている間だけ [`accent_focus`] の縁と、焦点色を
/// ひとさじ混ぜた面が出る。
///
/// 当たっていない間は枠を出さない——「いま入力を受けている場所」を示すのが
/// この縁の唯一の仕事で、常時出していると合図として働かない。
fn center_frame_style(active: bool) -> container::Style {
    let background = surface_card();
    let radius = palette::radius_card();
    if !active {
        return panel_style(background, radius);
    }
    let mut style = panel_style(palette::mix(background, accent_focus(), 0.06), radius);
    style.border = Border {
        color: palette::with_alpha(accent_focus(), 0.72),
        width: 1.0,
        radius: radius.into(),
    };
    style.shadow = iced::Shadow {
        color: palette::with_alpha(accent_focus(), 0.2),
        offset: iced::Vector::default(),
        blur_radius: 5.0,
    };
    style
}

fn tick_focus_flash(flash: Option<FocusFlash>) -> Option<FocusFlash> {
    flash.and_then(|flash| {
        (flash.ticks_remaining > 1).then_some(FocusFlash {
            ticks_remaining: flash.ticks_remaining - 1,
            ..flash
        })
    })
}

fn focus_flash_is_active(flash: Option<FocusFlash>, pane: FocusPane) -> bool {
    flash.is_some_and(|flash| flash.pane == pane && flash.ticks_remaining > 0)
}

/// 中央を借りているパネルが道を譲る用事(ほかの画面・タブへ移る操作)。
fn center_yields_to(message: &Message) -> bool {
    matches!(
        message,
        Message::SessionPressed(_)
            | Message::Shortcut(
                Shortcut::Terminal
                    | Shortcut::Claude
                    | Shortcut::Shell
                    | Shortcut::Next
                    | Shortcut::Previous
                    | Shortcut::Browser
                    | Shortcut::Help
                    | Shortcut::Appearance
            )
    )
}

/// 設定の画面が出ている間の素のキー。L/H で言語、J/K で配色。
fn settings_key(key: &panel::KeyPress) -> Option<settings::Message> {
    match key.latin().filter(|_| key.modifiers.is_empty()) {
        Some('l') => Some(settings::turn_language(true)),
        Some('h') => Some(settings::turn_language(false)),
        Some('j') => Some(settings::turn_theme(true)),
        Some('k') => Some(settings::turn_theme(false)),
        _ => None,
    }
}






fn load_fonts() -> Task<Message> {
    let mut bytes = font::load_essential();
    // 時計の字: macOS 同梱の Helvetica Neue をそのまま登録する
    if let Ok(b) = std::fs::read("/System/Library/Fonts/HelveticaNeue.ttc") {
        bytes.push(b);
    }
    Task::batch(
        bytes
            .into_iter()
            .map(|bytes| iced::font::load(bytes).discard::<()>()),
    )
    .collect()
    .discard::<Message>()
    .chain(Task::done(Message::FontsLoaded))
}

/// Italic 2 本の後追い登録。端末が立ったあとに呼ぶ。
fn load_late_fonts() -> Task<Message> {
    Task::batch(
        font::load_late()
            .into_iter()
            .map(|bytes| iced::font::load(bytes).discard::<()>()),
    )
    .collect()
    .discard::<Message>()
    .chain(Task::done(Message::LateFontsLoaded))
}

/// 非同期の一覧更新が、切り替え後の選択を古い状態へ戻すのを防ぐ。
#[derive(Default)]
struct TabSync {
    generation: u64,
    pending: Option<u64>,
    snapshot: Option<Vec<armature_core::monitor::LocalTab>>,
}

impl TabSync {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn begin(&mut self) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        self.pending = Some(self.generation);
        Some(self.generation)
    }

    fn accept(&mut self, generation: u64, tabs: &[armature_core::monitor::LocalTab]) -> bool {
        if self.pending != Some(generation) {
            return false;
        }
        self.pending = None;
        if generation != self.generation {
            return false;
        }
        self.snapshot = Some(tabs.to_vec());
        true
    }

}

fn open_browser(source: browser::Source, size: Size, scale: f32) -> Task<Message> {
    let bounds = browser_bounds(size, scale);
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            let source = source.clone();
            iced::window::run(id, move |window| browser::open(window, source, bounds))
                .map(Message::BrowserState)
        }
        None => Task::done(Message::BrowserState(Err(
            tr!("No window to hold the browser", "中央ブラウザを載せるウィンドウが見つからない").into(),
        ))),
    })
}

fn revive_browser(
    tabs: Vec<browser::TabInfo>,
    active: usize,
    size: Size,
    scale: f32,
) -> Task<Message> {
    let bounds = browser_bounds(size, scale);
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            let tabs = tabs.clone();
            iced::window::run(id, move |window| {
                browser::revive(window, tabs, active, bounds)
            })
            .map(Message::BrowserState)
        }
        None => Task::none(),
    })
}

/// アドレス欄に打った字の行き先へ、いま見ているタブを差し替える。
fn navigate_browser(typed: String) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            let typed = typed.clone();
            iced::window::run(id, move |_| browser::navigate_active(&typed))
                .map(Message::BrowserState)
        }
        None => Task::none(),
    })
}

/// 窓そのものへ鍵を戻す。WebView は Iced の外に浮いていて、そちらが鍵を
/// 持っている間は Iced 側の欄に焦点が入らない。
fn window_focus() -> Task<Message> {
    iced::window::latest().then(|id| match id {
        Some(id) => iced::window::gain_focus(id),
        None => Task::none(),
    })
}

fn browser_history(delta: i32) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => iced::window::run(id, move |_| browser::history_step(delta))
            .map(Message::BrowserState),
        None => Task::none(),
    })
}

fn resize_browser(size: Size, scale: f32) -> Task<Message> {
    let bounds = browser_bounds(size, scale);
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            iced::window::run(id, move |_| browser::resize(bounds)).map(Message::BrowserChanged)
        }
        None => Task::none(),
    })
}

/// 窓の実寸を測り直す。返事は [`Message::WindowMeasured`]。
fn measure_window() -> Task<Message> {
    iced::window::latest().then(|id| match id {
        Some(id) => iced::window::size(id).map(Message::WindowMeasured),
        None => Task::none(),
    })
}

/// 頁の枠が窓とずれていたら置き直す。ずれていなければ何もしない
/// ——毎秒通る道なので、揃っている間は WebView を1枚も叩かない。
fn reconcile_browser(bounds: browser::Bounds) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            iced::window::run(id, move |_| browser::reconcile(bounds)).map(Message::BrowserChanged)
        }
        None => Task::none(),
    })
}

fn zoom_browser(scale: f32) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            iced::window::run(id, move |_| browser::set_zoom(scale)).map(Message::BrowserChanged)
        }
        None => Task::none(),
    })
}

fn show_browser(size: Size, scale: f32) -> Task<Message> {
    let bounds = browser_bounds(size, scale);
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            iced::window::run(id, move |_| browser::show(bounds)).map(Message::BrowserState)
        }
        None => Task::none(),
    })
}

fn hide_browser() -> Task<Message> {
    run_browser_state(browser::hide)
}

/// 再起動の申し送りを置く場所。状態の置き場の下——窓の姿の話なので、
/// タスクの置き場には出さない。
fn restart_mode_path() -> std::path::PathBuf {
    armature_core::geo::state_dir().join("cockpit/restart.conf")
}

/// ▶ Restart を押した窓が「次の窓が開くまで待っている」印。置くのは古い窓、
/// 消すのは次の窓の `Opened`。中身は置いた側の pid(読み手はいない・目視用)。
fn restart_handoff_path() -> std::path::PathBuf {
    armature_core::geo::state_dir().join("cockpit/restart.handoff")
}

/// 次の窓を待つ上限。これを超えたら従来どおり落ちる——次の窓が起きられなかった
/// ときに古い窓が居座り続けるより、今までの挙動に戻るほうがまし。
const RESTART_HANDOFF_TIMEOUT: Duration = Duration::from_secs(10);

fn place_restart_handoff() {
    let path = restart_handoff_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, format!("{}\n", std::process::id()));
}

fn clear_restart_handoff() {
    let _ = std::fs::remove_file(restart_handoff_path());
}

/// 印が消える(=次の窓が `Opened` を受けた)まで待つ。消えたら true、
/// 上限まで消えなければ false。呼ぶ側はどちらでも落ちる。
fn wait_for_restart_handoff(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !path.exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 窓の姿と居場所の控え。札(`RestartNote`)と違い、読んでも消えない——起こし方に
/// 関わらず「前はどこで、どんな姿で開いていたか」を答える1件。
///
/// **居場所まで控えるのは、全画面が窓の乗っている画面で起きるから。** 姿だけ
/// 覚えて位置を Centered に戻すと、主画面で全画面になる。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct WindowPlace {
    fullscreen: bool,
    position: Option<iced::Point>,
}

fn window_place_path() -> PathBuf {
    armature_core::geo::state_dir().join("armature-window")
}

fn save_window_place(place: WindowPlace) {
    let path = window_place_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut text = String::new();
    if place.fullscreen {
        text.push_str("mode=fullscreen\n");
    }
    if let Some(point) = place.position {
        text.push_str(&format!("x={}\ny={}\n", point.x, point.y));
    }
    let _ = std::fs::write(path, text);
}

fn load_window_place() -> WindowPlace {
    let Ok(text) = std::fs::read_to_string(window_place_path()) else {
        return WindowPlace::default();
    };
    let mut place = WindowPlace::default();
    let (mut x, mut y) = (None, None);
    for line in text.lines() {
        let line = line.trim();
        if line == "mode=fullscreen" {
            place.fullscreen = true;
        } else if let Some(value) = line.strip_prefix("x=") {
            x = value.parse::<f32>().ok();
        } else if let Some(value) = line.strip_prefix("y=") {
            y = value.parse::<f32>().ok();
        }
    }
    if let (Some(x), Some(y)) = (x, y) {
        place.position = Some(iced::Point::new(x, y));
    }
    place
}

/// 姿を控え直すまでの間。
const MODE_NOTE_GAP: std::time::Duration = std::time::Duration::from_millis(400);

/// ▶ Restart を押した窓が次の窓へ渡す申し送り。
///
/// **持っていくのは「どう立つか」だけ。** 中身(観ていた所・開いていたタブ)は
/// それぞれの正本が既に控えているので、ここに写しを作らない。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RestartNote {
    /// 全画面のまま押されたか。
    fullscreen: bool,
}

impl RestartNote {
    fn is_empty(&self) -> bool {
        !self.fullscreen
    }
}

/// ▶ Restart を押した窓の申し送りを焼く。渡すものが無ければ札そのものを消す
/// ——古い札が残っていると、次の窓が理由もなく全画面で立つ。
fn store_restart_note(note: &RestartNote) {
    let path = restart_mode_path();
    if note.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, render_restart_note(note));
}

/// 札を読み、その場で消す。読めなければ何も無かったことにする。
fn take_restart_note() -> RestartNote {
    let path = restart_mode_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return RestartNote::default();
    };
    let _ = std::fs::remove_file(&path);
    parse_restart_note(&text)
}

fn render_restart_note(note: &RestartNote) -> String {
    let mut text = String::new();
    if note.fullscreen {
        text.push_str("mode=fullscreen\n");
    }
    text
}

fn parse_restart_note(text: &str) -> RestartNote {
    let mut note = RestartNote::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("mode=") {
            note.fullscreen = value.trim() == "fullscreen";
        }
    }
    note
}

fn restart_current_executable() -> Result<(), String> {
    let current = std::env::current_exe()
        .map_err(|error| format!("can't find the running app: {error}"))?;
    let target = restart_target(current)?;
    // **束があれば `open` に渡す。** 実体を直に起こすと Launch Services を通らず、
    // macOS は前面へ出す相手として扱わない——新しい窓が後ろに沈み、利用者は
    // 消えた窓を Dock から拾い直すことになる。束の無い実体(`cargo run`)は
    // 下の直起こしへ落とす。
    if let Some(bundle) = enclosing_bundle(&target) {
        let opened = std::process::Command::new("/usr/bin/open")
            .arg("-n")
            .arg(&bundle)
            .status()
            .is_ok_and(|status| status.success());
        if opened {
            return Ok(());
        }
    }
    armature_core::proc::child_env(&mut std::process::Command::new(&target))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("can't restart {}: {error}", target.display()))?;
    Ok(())
}

/// 実体を包む `.app`。`…/Cockpit.app/Contents/MacOS/armature` から3つ上。
fn enclosing_bundle(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension()? == "app").then(|| bundle.to_path_buf())
}

fn current_binary_fingerprint() -> Option<BinaryFingerprint> {
    std::env::current_exe().ok().and_then(binary_fingerprint)
}

fn binary_fingerprint(path: std::path::PathBuf) -> Option<BinaryFingerprint> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::metadata(&path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| (duration.as_secs(), duration.subsec_nanos()));
    Some(BinaryFingerprint {
        path,
        device: metadata.dev(),
        inode: metadata.ino(),
        len: metadata.len(),
        modified,
    })
}

fn binary_was_replaced(
    startup: Option<&BinaryFingerprint>,
    current: Option<&BinaryFingerprint>,
) -> bool {
    startup
        .zip(current)
        .is_some_and(|(startup, current)| startup.path == current.path && startup != current)
}

/// 起こし直す実体。**名前は照合しない**——外の crate の版は束の実体の名を
/// `BINARY_NAME` で替えるので、`armature` の名で決め打つとその版だけ起こし直せない。
/// 今動いている実体そのもの(`current_exe`)を、在ることだけ確かめて使う。
fn restart_target(current: std::path::PathBuf) -> Result<std::path::PathBuf, String> {
    if !current.is_absolute() || !current.is_file() {
        return Err(format!(
            "not restarting: can't find the running binary: {}",
            current.display()
        ));
    }
    Ok(current)
}

fn select_browser(index: usize) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            iced::window::run(id, move |_| browser::select(index)).map(Message::BrowserState)
        }
        None => Task::none(),
    })
}

fn cycle_browser(step: i32) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => iced::window::run(id, move |_| browser::cycle(step)).map(Message::BrowserState),
        None => Task::none(),
    })
}

fn close_browser_tab() -> Task<Message> {
    run_browser_state(browser::close_active)
}

fn focus_browser() -> Task<Message> {
    run_browser_effect(browser::focus)
}

fn blur_browser() -> Task<Message> {
    run_browser_effect(browser::blur)
}

fn update_browser_metadata(
    tab_id: u64,
    title: Option<String>,
    url: Option<String>,
) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            let title = title.clone();
            let url = url.clone();
            iced::window::run(id, move |_| Ok(browser::update(tab_id, title, url)))
                .map(Message::BrowserState)
        }
        None => Task::none(),
    })
}

fn translate_browser(
    id: u64,
    nonce: String,
    ask: u32,
    claude: bool,
    texts: Vec<String>,
) -> Task<Message> {
    Task::perform(
        async move {
            let fallback_nonce = nonce.clone();
            tokio::task::spawn_blocking(move || browser::translate(id, nonce, ask, claude, texts))
                .await
                .unwrap_or_else(|error| browser::TranslationReply {
                    id,
                    script: format!(
                        "window.__cockpitTr && window.__cockpitTr.take({}, {ask}, false, [], {});",
                        serde_json::to_string(&fallback_nonce).unwrap_or_else(|_| "\"\"".into()),
                        serde_json::to_string(&tr!(
                            format!("The translation thread ended: {error}"),
                            format!("翻訳スレッドが終了した: {error}"),
                        ))
                        .unwrap_or_else(|_| "\"translation failed\"".into())
                    ),
                })
        },
        Message::BrowserTranslation,
    )
}

fn deliver_translation(reply: browser::TranslationReply) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => {
            let reply = reply.clone();
            iced::window::run(id, move |_| browser::deliver(&reply)).map(Message::BrowserChanged)
        }
        None => Task::none(),
    })
}

fn run_browser_state(action: fn() -> Result<browser::Snapshot, String>) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => iced::window::run(id, move |_| action()).map(Message::BrowserState),
        None => Task::none(),
    })
}

fn run_browser_effect(action: fn() -> Result<(), String>) -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => iced::window::run(id, move |_| action()).map(Message::BrowserChanged),
        None => Task::none(),
    })
}

fn open_browser_bookmark(id: u64, nonce: u64, url: String) -> Task<Message> {
    iced::window::latest().then(move |window_id| match window_id {
        Some(window_id) => {
            let url = url.clone();
            iced::window::run(window_id, move |_| browser::open_bookmark(id, nonce, &url))
                .map(Message::BrowserChanged)
        }
        None => Task::none(),
    })
}

fn delete_browser_bookmark(id: u64, nonce: u64, spec: String) -> Task<Message> {
    iced::window::latest().then(move |window_id| match window_id {
        Some(window_id) => {
            let spec = spec.clone();
            iced::window::run(window_id, move |_| {
                browser::delete_bookmark(id, nonce, &spec)
            })
            .map(Message::BrowserChanged)
        }
        None => Task::none(),
    })
}

fn configure_native_shortcuts() -> Task<Message> {
    iced::window::latest().then(move |id| match id {
        Some(id) => iced::window::run(id, move |window| {
            browser::install_native_shortcuts();
            use objc2::{MainThreadMarker, MainThreadOnly};
            use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu};
            use objc2_foundation::NSString;

            fn clear_command_h(menu: &NSMenu, empty: &NSString) {
                for item in menu.itemArray().iter() {
                    if item.keyEquivalent().to_string() == "h"
                        && item.keyEquivalentModifierMask() == NSEventModifierFlags::Command
                    {
                        item.setKeyEquivalent(empty);
                    }
                    if let Some(submenu) = item.submenu() {
                        clear_command_h(&submenu, empty);
                    }
                }
            }

            /// 編集メニューを組む。
            ///
            /// **⌘系はまずメニューのキー等価物としてレスポンダ連鎖へ配られる。**
            /// 編集メニューが無いと、頁が自前で拾わない場面で ⌘C が誰にも届かず
            /// 落ちる——「たまに効かない」の正体。
            ///
            /// ⌘V だけは載せない。字の貼り付けは端末の受け持ちで、こちらが
            /// メニューで横取りすると端末の貼り付けまで黙る(2026-08-17に一度
            /// そうして壊した)。画像の貼り付けは `Shortcut::Paste` が受ける。
            fn install_edit_menu(menu: &NSMenu, mtm: MainThreadMarker) {
                let title = NSString::from_str(tr!("Edit", "編集"));
                if menu
                    .itemArray()
                    .iter()
                    .any(|item| matches!(item.title().to_string().as_str(), "Edit" | "編集"))
                {
                    return;
                }
                let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
                let entries: [(&str, objc2::runtime::Sel, &str, NSEventModifierFlags); 5] = [
                    (
                        tr!("Undo", "取り消す"),
                        objc2::sel!(undo:),
                        "z",
                        NSEventModifierFlags::Command,
                    ),
                    (
                        tr!("Redo", "やり直す"),
                        objc2::sel!(redo:),
                        "z",
                        NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift),
                    ),
                    (
                        tr!("Cut", "カット"),
                        objc2::sel!(cut:),
                        "x",
                        NSEventModifierFlags::Command,
                    ),
                    (
                        tr!("Copy", "コピー"),
                        objc2::sel!(copy:),
                        "c",
                        NSEventModifierFlags::Command,
                    ),
                    (
                        tr!("Select All", "すべてを選択"),
                        objc2::sel!(selectAll:),
                        "a",
                        NSEventModifierFlags::Command,
                    ),
                ];
                for (name, action, key, mask) in entries {
                    // SAFETY: どれも AppKit が標準で持つ編集の action。target は
                    // 置かない——nil のままにすると、いま鍵を持っている相手
                    // (WebView・テキスト欄)まで連鎖で届く。
                    let item = unsafe {
                        edit.addItemWithTitle_action_keyEquivalent(
                            &NSString::from_str(name),
                            Some(action),
                            &NSString::from_str(key),
                        )
                    };
                    item.setKeyEquivalentModifierMask(mask);
                }
                // SAFETY: 見出しだけの項目。action を持たせないので何も起きない。
                let holder = unsafe {
                    menu.addItemWithTitle_action_keyEquivalent(&title, None, &NSString::from_str(""))
                };
                holder.setSubmenu(Some(&edit));
            }

            if let Some(mtm) = MainThreadMarker::new() {
                let app = NSApplication::sharedApplication(mtm);
                if let Some(menu) = app.mainMenu() {
                    let empty = NSString::from_str("");
                    clear_command_h(&menu, &empty);
                    install_edit_menu(&menu, mtm);
                }
                // ⌘Q と Dock の「終了」も閉じる釦と同じ道へ(パネルの on_exit を通す)。
                if let Some(ns_window) = ns_window_of(window) {
                    quit::route_to_close(ns_window, mtm);
                }
            }
        })
        .map(|_| Message::NativeShortcutsReady),
        None => Task::none(),
    })
}

/// iced の窓の NSWindow。
fn ns_window_of(
    window: &dyn iced::window::Window,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use iced::window::raw_window_handle::RawWindowHandle;

    let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: winit が渡す AppKit の窓の NSView。窓の糸で、窓が生きている間に読む。
    let view: &objc2_app_kit::NSView = unsafe { handle.ns_view.cast().as_ref() };
    view.window()
}

/// 中央の WebView を置く枠。
///
/// `size` は Iced の論理寸法で、拡大率のぶんだけ実寸より小さい。wry が受ける
/// のは AppKit のポイント(実寸)なので、組み上げた枠へ最後に拡大率を掛けて戻す。
/// アドレスバーの帯の高さ(論理px)。[`Cockpit::browser_chrome`] と対。
///
/// 帯の中身(高さ24の欄)に上下の余白5を足した数。余白を削ると欄の枠が
/// 中央のパネルの枠線と重なって見える。
const BROWSER_CHROME_HEIGHT: f64 = 34.0;


mod icon {
    pub const RESTART: &str = "\u{f021}";
}

/// ブラウザの伝言(IPC)の見張り。50ms ごとに覗き、**用事があるときだけ**便を出す。
fn browser_request_watch() -> impl futures::Stream<Item = Message> {
    iced::stream::channel(4, |mut out: futures::channel::mpsc::Sender<Message>| async move {
        use futures::SinkExt;
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if browser::has_requests() && out.send(Message::BrowserPoll).await.is_err() {
                break;
            }
        }
    })
}

fn browser_bounds(size: Size, scale: f32) -> browser::Bounds {
    // **パネルの並びと同じ寸法を使う**([`geometry`])。WebView は Iced の外に浮いていて
    // 自分では配置されないので、ここがパネルの並びと食い違うと中央だけ枠から外れる。
    bounds_in(geometry(), size, scale)
}

fn bounds_in(geo: Geometry, size: Size, scale: f32) -> browser::Bounds {
    let padding_x = f64::from(geo.pad_x);
    let padding_y = f64::from(geo.pad_y);
    let gap = f64::from(geo.gap_x);
    // 枠の1pxに、頁と枠の間の余白を足した数。**1pxちょうどだと枠が頁に
    // 舐められて消える**。
    // 明るい頁の上では、線そのものより線の両側の暗い余白が枠に見える。
    const FRAME: f64 = 3.0;
    let left = geo.available(size.width) * f64::from(geo.left) / geo.shares();
    let left_gap = if geo.left > 0 { gap } else { 0.0 };
    let center = geo.available(size.width) * f64::from(geo.centre) / geo.shares();
    let scale = f64::from(scale);
    browser::Bounds {
        x: (padding_x + left + left_gap + FRAME) * scale,
        y: (padding_y + FRAME + BROWSER_CHROME_HEIGHT) * scale,
        width: (center - FRAME * 2.0).max(0.0) * scale,
        height: (f64::from(size.height)
            - padding_y * 2.0
            - FRAME * 2.0
            - BROWSER_CHROME_HEIGHT)
            .max(0.0)
            * scale,
    }
}


/// アイコンの字。**自分で描かない**——束はすでに JetBrains Mono Nerd Font を
/// 予備の書体として読んでいて(`font.rs` の `USER_FALLBACK`)、Font Awesome の
/// 枠がそのまま出る。

#[cfg(test)]
mod tests {
    use super::*;

    /// 切り替えの前に取り始めたタブの列挙は捨てる。取得中に続けて ⌃Tab を押しても、
    /// 最後に確かめた前面が古い結果で巻き戻らない。
    #[test]
    fn tab_sync_drops_results_started_before_a_switch() {
        let tab = |target: &str, active| armature_core::monitor::LocalTab {
            target: target.into(),
            active,
            ..Default::default()
        };
        let mut sync = TabSync::default();
        let before_switch = sync.begin().unwrap();
        sync.invalidate();
        sync.invalidate();
        assert!(sync.begin().is_none(), "one at a time");
        assert!(!sync.accept(before_switch, &[tab("a", true), tab("b", false)]));
        let after_switch = sync.begin().unwrap();
        assert!(sync.accept(after_switch, &[tab("b", true), tab("c", false)]));
        assert!(!sync.accept(before_switch, &[tab("a", true)]));
        let next = sync.begin().unwrap();
        assert!(sync.accept(next, &[tab("b", false), tab("c", true)]));
    }

    #[test]
    fn the_welcome_page_names_this_build_and_its_folder() {
        let home = "/Users/someone";
        assert_eq!(shown_path(Path::new("/Users/someone/My Armature"), home), "~/My Armature");
        assert_eq!(shown_path(Path::new("/Users/someone"), home), "~");
        // 名前が前方一致するだけの別のホームは縮めない。
        assert_eq!(shown_path(Path::new("/Users/someone2/Armature"), home), "/Users/someone2/Armature");
        assert_eq!(shown_path(Path::new("/Volumes/x/Armature"), home), "/Volumes/x/Armature");

        for page in [include_str!("../assets/welcome.html"), include_str!("../assets/welcome-ja.html")] {
            let html = personalize_welcome(page, "~/My <Armature>", "My <Armature>");
            assert!(!html.contains("~/Armature"));
            assert!(html.contains("~/My &lt;Armature&gt;/tasks/"));
            assert!(html.contains("— My &lt;Armature&gt;</title>"));
            // 見た目の設定は削ってある。頁が無い設定を案内しない。
            assert!(!html.contains("look,") && !html.contains("見た目"));
        }
    }

    #[test]
    fn the_external_open_mark_is_a_line_drawing_tinted_like_its_neighbours() {
        // 書体の字で出すと隣の ‹ › ⟳ より太く大きく浮いた。
        // 線画の色は `currentColor` を差し替えて字と同じ text_muted にする。
        let svg = include_str!("../assets/icons/external-link.svg");
        assert!(svg.contains("stroke=\"currentColor\""));
        assert!(svg.contains("fill=\"none\""));
    }




    #[test]
    fn a_center_closes_for_navigation_but_survives_ticks() {
        assert!(center_yields_to(&Message::Shortcut(Shortcut::Terminal)));
        assert!(center_yields_to(&Message::SessionPressed("tmux:1".into())));
        assert!(center_yields_to(&Message::Shortcut(Shortcut::Browser)));
        assert!(!center_yields_to(&Message::Tick));
        assert!(!center_yields_to(&Message::PanelPressed(0)));
    }


    #[test]
    fn a_rewritten_file_gets_a_new_stamp() {
        let path =
            std::env::temp_dir().join(format!("armature-stamp-{}.html", std::process::id()));
        std::fs::write(&path, "<p>1</p>").expect("下ごしらえは書ける");
        let first = file_stamp(&path).expect("刻が取れる");
        // 触っていない間は同じ刻——毎秒の見張りが空振りで載せ替えない。
        assert_eq!(file_stamp(&path), Some(first));
        std::fs::write(&path, "<p>1</p><p>2</p>").expect("書き替えられる");
        assert_ne!(
            file_stamp(&path),
            Some(first),
            "書き替えたのに刻が動いていない"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_that_is_not_there_has_no_stamp() {
        // 消された資料は載せ替えの相手にしない(刻が無い回は次の周期へ回す)。
        assert!(file_stamp(Path::new("/tmp/armature-absolutely-absent")).is_none());
    }



    /// 時刻は2桁で揃える。


    #[test]
    fn a_bundled_restart_goes_through_the_app_so_the_new_window_comes_up_in_front() {
        let bundled = std::path::Path::new(
            "/Applications/Armature.app/Contents/MacOS/armature",
        );
        assert_eq!(
            enclosing_bundle(bundled),
            Some(std::path::PathBuf::from("/Applications/Armature.app"))
        );
        // cargo が置く実体は束の中に居ない。直に起こす側へ落ちること。
        assert_eq!(
            enclosing_bundle(std::path::Path::new(
                "/Users/x/dev/armature/rust/target/release/armature"
            )),
            None
        );
    }

    #[test]
    fn a_fullscreen_window_hands_the_next_window_its_mode() {
        let note = RestartNote { fullscreen: true };
        assert_eq!(parse_restart_note(&render_restart_note(&note)), note);
        assert!(RestartNote::default().is_empty());
        assert!(render_restart_note(&RestartNote::default()).is_empty());
        assert!(!parse_restart_note("mode=windowed\n").fullscreen);
        assert_eq!(parse_restart_note(""), RestartNote::default());
    }

    #[test]
    fn restart_handoff_waits_until_the_next_window_clears_it() {
        let dir = std::env::temp_dir().join(format!("cockpit-handoff-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("restart.handoff");
        std::fs::write(&path, "1\n").unwrap();
        // 次の窓の役: 少し経ってから印を消す。
        let remover = {
            let path = path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                let _ = std::fs::remove_file(&path);
            })
        };
        let started = Instant::now();
        assert!(wait_for_restart_handoff(&path, Duration::from_secs(5)));
        assert!(started.elapsed() < Duration::from_secs(5));
        remover.join().unwrap();
        // 誰も消さなければ上限で諦めて false。
        std::fs::write(&path, "1\n").unwrap();
        assert!(!wait_for_restart_handoff(&path, Duration::from_millis(120)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restart_takes_the_running_binary_whatever_its_name() {
        let root =
            std::env::temp_dir().join(format!("cockpit-restart-target-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("armature");
        std::fs::write(&target, b"test").unwrap();
        assert_eq!(restart_target(target.clone()).unwrap(), target);
        // make-app.sh の BINARY_NAME で名を替えた版も起こし直せる。
        let renamed = root.join("my-armature");
        std::fs::write(&renamed, b"test").unwrap();
        assert_eq!(restart_target(renamed.clone()).unwrap(), renamed);
        assert!(restart_target(root.join("gone")).is_err());
        assert!(restart_target(std::path::PathBuf::from("relative/armature")).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn restart_is_off_until_the_same_binary_path_is_replaced() {
        let root = std::env::temp_dir().join(format!(
            "cockpit-restart-fingerprint-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("armature");
        std::fs::write(&path, b"old").unwrap();
        let startup = binary_fingerprint(path.clone()).unwrap();
        assert!(!binary_was_replaced(Some(&startup), Some(&startup)));

        std::fs::write(&path, b"new binary body").unwrap();
        let replaced = binary_fingerprint(path).unwrap();
        assert!(binary_was_replaced(Some(&startup), Some(&replaced)));

        let other = BinaryFingerprint {
            path: root.join("somewhere-else"),
            ..replaced
        };
        assert!(!binary_was_replaced(Some(&startup), Some(&other)));
        assert!(!binary_was_replaced(Some(&startup), None));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn command_plus_and_minus_step_the_zoom_within_its_bounds() {
        use iced::keyboard::key::{Code, Physical};

        let minus = Key::Character("-".into());
        let physical_minus = Physical::Code(Code::Minus);
        assert_eq!(
            shortcut_for_key(&minus, physical_minus, Modifiers::COMMAND),
            Some(Shortcut::ZoomOut)
        );
        // US 配列は ⌘= でも、JIS 配列は ⌘⇧+ でも同じ拡大へ届く。
        assert_eq!(
            shortcut_for_key(
                &Key::Character("=".into()),
                Physical::Code(Code::Equal),
                Modifiers::COMMAND
            ),
            Some(Shortcut::ZoomIn)
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Character("+".into()),
                Physical::Code(Code::Equal),
                Modifiers::COMMAND | Modifiers::SHIFT
            ),
            Some(Shortcut::ZoomIn)
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Character("0".into()),
                Physical::Code(Code::Digit0),
                Modifiers::COMMAND
            ),
            Some(Shortcut::ZoomReset)
        );
        // 修飾なしの `-` は端末へ通す。
        assert_eq!(
            shortcut_for_key(&minus, physical_minus, Modifiers::empty()),
            None
        );

        // 上下限を超えて刻んでも止まる。
        assert!((font::stepped_scale(font::MAX_SCALE, 1) - font::MAX_SCALE).abs() < 1e-6);
        assert!((font::stepped_scale(font::MIN_SCALE, -1) - font::MIN_SCALE).abs() < 1e-6);
    }

    #[test]
    fn the_center_webview_keeps_its_place_when_the_zoom_changes() {
        // 拡大率が変わっても窓の実寸は変わらない。論理寸法を割り戻した枠が
        // 実寸のポイントへ戻ることを確かめる(戻さないと WebView がずれる)。
        let scale = 1.5;
        let scaled = browser_bounds(
            Size::new(WINDOW_WIDTH / scale, WINDOW_HEIGHT / scale),
            scale,
        );
        let plain = browser_bounds(Size::new(WINDOW_WIDTH, WINDOW_HEIGHT), 1.0);
        // パネルの取り分は割合なので中央の幅はほぼ変わらず、余白と枠のぶんだけ縮む。
        assert!((scaled.width - plain.width).abs() < 20.0);
        assert!(scaled.x > plain.x);
        assert!(scaled.height < plain.height);
        assert!(scaled.width > 0.0 && scaled.height > 0.0);
    }


    #[test]
    fn terminal_shortcuts_choose_the_requested_tab_action() {
        use iced::keyboard::key::{Code, Physical};

        let t = Key::Character("t".into());
        let physical_t = Physical::Code(Code::KeyT);
        assert_eq!(
            shortcut_for_key(&t, physical_t, Modifiers::COMMAND),
            Some(Shortcut::Claude)
        );
        assert_eq!(
            shortcut_for_key(&t, physical_t, Modifiers::COMMAND | Modifiers::SHIFT),
            Some(Shortcut::Reopen)
        );
        // シェルは ⌘J へ移した(⇧⌘T をタブの復活へ譲ったため)。
        assert_eq!(
            shortcut_for_key(
                &Key::Character("j".into()),
                Physical::Code(Code::KeyJ),
                Modifiers::COMMAND
            ),
            Some(Shortcut::Shell)
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Named(Named::Tab),
                Physical::Code(Code::Tab),
                Modifiers::CTRL
            ),
            Some(Shortcut::Next)
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Named(Named::Tab),
                Physical::Code(Code::Tab),
                Modifiers::CTRL | Modifiers::SHIFT
            ),
            Some(Shortcut::Previous)
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Character("w".into()),
                Physical::Code(Code::KeyW),
                Modifiers::COMMAND
            ),
            Some(Shortcut::Close)
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Character("n".into()),
                Physical::Code(Code::KeyN),
                Modifiers::COMMAND
            ),
            None,
            "⌘N は器が取らない(タスクは Claude に頼んで足す)"
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Character("m".into()),
                Physical::Code(Code::KeyM),
                Modifiers::COMMAND
            ),
            None,
            "⌘M は器が取らない(音楽はパネルの部品)"
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Character("h".into()),
                Physical::Code(Code::KeyH),
                Modifiers::COMMAND
            ),
            Some(Shortcut::Center)
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Character("l".into()),
                Physical::Code(Code::KeyL),
                Modifiers::COMMAND
            ),
            Some(Shortcut::Right)
        );
        // ⌘ , は3通りのどれで来ても開く。日本語入力が有効だと論理キーが読点に
        // なり、`to_latin` の表(英字と数字だけ)では拾えない
        // 。
        for key in [
            Key::Character(",".into()),
            Key::Character("、".into()),
            Key::Character("¿".into()),
        ] {
            assert_eq!(
                shortcut_for_key(&key, Physical::Code(Code::Comma), Modifiers::COMMAND),
                Some(Shortcut::Appearance),
                "{key:?}"
            );
        }
        // 修飾なしの読点は素通し。端末へ字を打つ邪魔をしない。
        assert_eq!(
            shortcut_for_key(
                &Key::Character(",".into()),
                Physical::Code(Code::Comma),
                Modifiers::empty()
            ),
            None
        );
        assert_eq!(
            shortcut_for_key(
                &Key::Character("/".into()),
                Physical::Code(Code::Slash),
                Modifiers::COMMAND
            ),
            Some(Shortcut::Help)
        );
        // 素の字はパネルへ。字は物理キーで読むので、かな入力の「あ」も j として届く。
        let kana_j = panel::KeyPress {
            key: Key::Character("あ".into()),
            physical: Physical::Code(Code::KeyJ),
            modifiers: Modifiers::empty(),
        };
        assert!(kana_j.is('j'), "IMEの入力文字ではなく物理Jキーで読む");
    }


    #[test]
    fn plain_n_and_a_pasted_field_leave_the_focus_decision_to_the_app() {
        use iced::keyboard::key::{Code, Physical};

        let press = |letter: &str, code, modifiers| {
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: Key::Character(letter.into()),
                modified_key: Key::Character(letter.into()),
                physical_key: Physical::Code(code),
                location: iced::keyboard::Location::Standard,
                modifiers,
                text: Some(letter.into()),
                repeat: false,
            })
        };
        // 素の字は器が取らず、鍵を持っているパネルへ回す(持っていなければ update が捨てる)。
        assert!(matches!(
            shortcut_message(
                press("n", Code::KeyN, Modifiers::empty()),
                iced::event::Status::Captured,
                iced::window::Id::unique()
            ),
            Some(Message::PanelKey(key)) if key.is('n')
        ));
        // 入力欄が貼った ⌘V は端末へ直行させない。
        assert!(matches!(
            shortcut_message(
                press("v", Code::KeyV, Modifiers::COMMAND),
                iced::event::Status::Captured,
                iced::window::Id::unique()
            ),
            Some(Message::FieldPaste)
        ));
        assert!(matches!(
            shortcut_message(
                press("v", Code::KeyV, Modifiers::COMMAND),
                iced::event::Status::Ignored,
                iced::window::Id::unique()
            ),
            Some(Message::Shortcut(Shortcut::Paste))
        ));
    }

    /// Esc は器が取らずにパネルの口へ回す。中央を借りたパネル(大きな暦)を閉じるのに使い、
    /// 端末へはそのまま届く(端末は自分で鍵を読む)。
    #[test]
    fn plain_escape_goes_to_the_panels() {
        use iced::keyboard::key::{Code, Physical};

        let event = iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            modified_key: Key::Named(Named::Escape),
            physical_key: Physical::Code(Code::Escape),
            location: iced::keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        });
        assert!(matches!(
            shortcut_message(event, iced::event::Status::Ignored, iced::window::Id::unique()),
            Some(Message::PanelKey(key)) if key.key == Key::Named(Named::Escape)
        ));
    }












    #[test]
    fn terminal_clicks_reclaim_the_keys() {
        assert!(terminal_command_claims_focus(
            &iced_term::BackendCommand::SelectUpdate((1.0, 1.0))
        ));
        assert!(!terminal_command_claims_focus(
            &iced_term::BackendCommand::Write(vec![b'j'])
        ));
    }

    #[test]
    fn the_outermost_ground_never_draws_a_frame() {
        assert_eq!(window_style().border.width, 0.0, "窓の外周に線が出た");
        assert_eq!(window_style().background, Some(Background::Color(surface_window())));
    }




    #[test]
    fn the_column_proportions_are_one_three_one() {
        assert_eq!([SIDE_SHARE, CENTRE_SHARE], [20, 60]);
    }

    /// WebView は Iced の外に浮いていて自分では配置されない。パネルの並びと
    /// 同じ数から組めていることを、1440×900 の実寸で押さえる。
    ///
    /// 収支(窓の縁0・列の間の線1・枠と余白3・アドレスバーの帯34):
    /// 使える幅 = 1440 - 1×2(列の間の線) = 1438、
    /// 左 = 1438×20/100 = 287.6、中央 = 1438×60/100 = 862.8。
    /// x = 287.6 + 1 + 3、y = 3 + 34、幅 = 862.8 - 6、
    /// 高さ = 900 - 6 - 34(枠のぶん縮め、帯のぶん下げる)。
    #[test]
    fn browser_fills_the_center_panel_at_the_default_window_size() {
        let geo = Geometry {
            pad_x: 0.0,
            pad_y: 0.0,
            gap_x: LINE,
            gap_y: LINE,
            left: SIDE_SHARE,
            centre: CENTRE_SHARE,
            right: SIDE_SHARE,
        };
        let bounds = bounds_in(geo, Size::new(1440.0, 900.0), 1.0);
        assert_eq!(bounds.y, 3.0 + BROWSER_CHROME_HEIGHT);
        assert_eq!(bounds.height, 894.0 - BROWSER_CHROME_HEIGHT);
        assert!((bounds.x - 291.6).abs() < 0.01, "{}", bounds.x);
        assert!((bounds.width - 856.8).abs() < 0.01, "{}", bounds.width);
    }

    /// 窓は1枚の画面。縁を持たず、列と列・パネルとパネルの間は線1本ぶんだけ空く。
    #[test]
    fn the_window_is_one_screen_divided_by_single_lines() {
        let geo = geometry();
        assert_eq!((geo.pad_x, geo.pad_y), (0.0, 0.0));
        assert_eq!((geo.gap_x, geo.gap_y), (LINE, LINE));
        let style = window_style();
        assert_eq!(style.background, Some(Background::Color(surface_card())));
    }

    /// 時計のすぐ下の暦だけは線を引かない。ほかの並びには引く。
    #[test]
    fn only_the_calendar_under_the_clock_goes_without_a_line() {
        assert!(joined_keys("clock", "calendar"));
        assert!(!joined_keys("calendar", "clock"));
        assert!(!joined_keys("calendar", "tasks"));
        assert!(!joined_keys("clock", "tasks"));
    }






    #[test]
    fn the_window_subscription_drops_the_redraw_report() {
        let id = iced::window::Id::unique();
        let status = iced::event::Status::Ignored;

        // 「描いた」の報せをメッセージにすると、描く→報せ→作り直し→描く の輪ができる。
        let redraw = iced::Event::Window(iced::window::Event::RedrawRequested(Instant::now()));
        assert!(window_message(redraw, status, id).is_none());

        // 盤が実際に使う出来事は今までどおり通す。
        let resized = iced::Event::Window(iced::window::Event::Resized(Size::new(100.0, 50.0)));
        assert!(matches!(
            window_message(resized, status, id),
            Some(Message::WindowEvent(iced::window::Event::Resized(_)))
        ));
        let closing = iced::Event::Window(iced::window::Event::CloseRequested);
        assert!(matches!(
            window_message(closing, status, id),
            Some(Message::WindowEvent(iced::window::Event::CloseRequested))
        ));
    }

    /// 中央の枠は、フォーカスが当たっているときだけ縁を出す。
    ///
    /// 当たっていない間は面の高さと影で地から分かれていればよく、常時の縁は
    /// 「いまここが入力を受けている」という合図を鈍らせる。
    #[test]
    fn the_center_frame_shows_its_edge_only_while_focused() {
        let idle = center_frame_style(false);
        let focused = center_frame_style(true);

        assert_eq!(idle.border.width, 0.0);
        assert_eq!(idle.background, Some(Background::Color(surface_card())));
        assert_eq!(focused.border.width, 1.0);
        assert_eq!(focused.border.color.b, accent_focus().b);
    }

    /// パネルは地と同じ画面の色で、枠も角丸も持たない(境は線が引く)。
    #[test]
    fn seats_are_the_screen_itself() {
        let card = card_style();

        assert_eq!(card.background, Some(Background::Color(surface_window())));
        assert_eq!(card.border.width, 0.0);
        assert_eq!(card.border.radius, 0.0.into());
    }



    #[test]
    fn focus_flash_fades_one_tick_at_a_time() {
        let flash = Some(FocusFlash {
            pane: FocusPane::Right,
            ticks_remaining: 2,
        });
        let next = tick_focus_flash(flash);
        assert_eq!(
            next,
            Some(FocusFlash {
                pane: FocusPane::Right,
                ticks_remaining: 1,
            })
        );
        assert_eq!(tick_focus_flash(next), None);
    }

    #[test]
    fn focus_flash_only_highlights_its_target_pane() {
        let flash = Some(FocusFlash {
            pane: FocusPane::Center,
            ticks_remaining: 1,
        });
        assert!(focus_flash_is_active(flash, FocusPane::Center));
        assert!(!focus_flash_is_active(flash, FocusPane::Right));
    }

}
