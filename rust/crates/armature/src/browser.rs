use std::cell::RefCell;
use armature_core::tr;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

#[cfg(target_os = "macos")]
use block2::RcBlock;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};

use iced::window::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect as WryRect, WebView, WebViewBuilder};

/// WebView の保存領域(Cookie・履歴)の名札。今のコックピットの領域とは別物にする。
const DATA_STORE: [u8; 16] = *b"armature-webview";

/// 名乗り。WKWebView の既定 UA には `Version/…` と `Safari/…` が無く、
/// 対応ブラウザ判定はほぼ例外なくこの2つを見る——素のままだと会議や
/// グループウェアが「対応していません」を出す(2026-08-18実測)。
/// 数字は端末の Safari に合わせるが、判定側が見るのは綴りの有無なので、
/// OSが進んで古びても実害は出ない。
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.6 Safari/605.1.15";

/// 打った字が行き先に見えないときの逃がし先。
const SEARCH: &str = "https://www.google.com/search?q=";

#[derive(Clone, Debug)]
pub enum Source {
    Url(String),
    HtmlFile {
        title: String,
        html: String,
        url: String,
    },
    NewTab {
        serial: u64,
        html: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabInfo {
    pub id: u64,
    pub title: String,
    pub url: String,
    /// 復元で並べただけで、まだ WebView を持たない控え。選ぶと起きる。
    pub asleep: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub tabs: Vec<TabInfo>,
    pub active: usize,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Hide,
    FocusCenter,
    /// ⌘L。頁から鍵を外してアドレス欄へ渡す。
    FocusAddress,
    /// ⌘R。いま見ている頁を読み直す。
    Reload,
    CloseTab,
    NextTab,
    PreviousTab,
    NewTab(String),
    NewBlank,
    /// ⌘J。
    NewShell,
    /// ⇧⌘T。閉じた Claude タブを戻す。
    Reopen,
    /// ⌘,。
    Settings,
    Help,
    Bookmark,
    /// 二本指の横払い。負で戻る・正で進む。
    History(i32),
    /// 頁の中のリンク(target=_blank 含む)。いま見ているタブを行き先へ移す。
    NavigateActive(String),
    /// ⌘⏎。頁を畳んで中央の端末へ鍵を渡す。
    FocusTerminal,
    /// ⇧⌘R。窓ごと起こし直す。頁を見ている間も同じ指で押せるよう、
    /// ここから器へ渡す。**⌘R(読み直し)と地続きの指**にしてある。
    Restart,
    OpenBookmark {
        id: u64,
        nonce: u64,
        url: String,
    },
    DeleteBookmark {
        id: u64,
        nonce: u64,
        spec: String,
    },
    SelectTab(usize),
    Title {
        id: u64,
        title: String,
    },
    Location {
        id: u64,
        url: String,
    },
    Translate {
        id: u64,
        nonce: String,
        ask: u32,
        claude: bool,
        texts: Vec<String>,
    },
}

#[derive(Clone, Debug)]
pub struct TranslationReply {
    pub id: u64,
    pub script: String,
}

/// タブが抱える面。復元直後は WebView を持たない控えで、初めて選ばれた
/// ときに起きる——保存済みの 67 枚を一斉に起こすと WebKit のプロセスが
/// 70 本・1.5GB 生まれ、窓が立った直後の機体が地面を擦る(2026-09-02 実測)。
enum Surface {
    Live(WebView),
    Asleep,
}

struct Tab {
    id: u64,
    title: String,
    url: String,
    new_tab_nonce: Option<u64>,
    surface: Surface,
}

impl Tab {
    fn live(&self) -> Option<&WebView> {
        match &self.surface {
            Surface::Live(view) => Some(view),
            Surface::Asleep => None,
        }
    }

    fn asleep(&self) -> bool {
        self.live().is_none()
    }
}

#[derive(Default)]
struct Manager {
    tabs: Vec<Tab>,
    active: usize,
    visible: bool,
    next_id: u64,
    /// 頁の拡大率(パネルの幅と ⌘+/⌘- から引いた実効値)。未設定なら
    /// [`current_page_zoom`] が保存済みの倍率から引く。
    zoom: Option<f64>,
    /// 利用者が ⌘+/⌘- で選んだ窓の文字倍率。実効の拡大率はパネルの幅で頭を
    /// 押さえられるので、押された数はこちらに残す——resize のたびに
    /// 押さえ直すのに要る。
    scale: Option<f32>,
    /// いまパネルが持っている幅(点)。0 のあいだはパネルがまだ測れていない。
    width: f64,
    /// 控えのタブを起こすときの親窓とパネルの位置。窓を渡される口
    /// (open / revive / show / resize)が通るたびに覚え直す。
    parent: Option<RawWindowHandle>,
    bounds: Option<Bounds>,
}

static REQUESTS: LazyLock<Mutex<Vec<Request>>> = LazyLock::new(|| Mutex::new(Vec::new()));
static BOOKMARK_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static NEXT_NEW_TAB: AtomicU64 = AtomicU64::new(1);
static NEXT_BOOKMARK_WRITE: AtomicU64 = AtomicU64::new(1);
/// 頁を見ている間に利用者が最後に鍵を押した時刻(native の監視が書く)。外の頁が
/// 新しいタブを開けるのは、この直後の 1 回だけ(リンクの札を ⇧ で選んだとき)。
static KEY_DOWN_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// ⇧T で精訳を頼まれたタブと、残りの束の数。頁の JS は本文を 20 塊ずつ送るので、
/// 1 回の ⇧T で頁 1 枚ぶん(最大 400 塊)まで通す。
static CLAUDE_ARM: Mutex<Option<(u64, u32)>> = Mutex::new(None);
const CLAUDE_ARM_BATCHES: u32 = 20;

thread_local! {
    static MANAGER: RefCell<Manager> = RefCell::new(Manager::default());
    #[cfg(target_os = "macos")]
    static NATIVE_SHORTCUT_MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NativeModifiers {
    command: bool,
    control: bool,
    option: bool,
    shift: bool,
}

#[cfg(target_os = "macos")]
fn native_request(key: &str, key_code: u16, modifiers: NativeModifiers) -> Option<Request> {
    let key = key.to_ascii_lowercase();
    if modifiers.control && !modifiers.command && !modifiers.option && key_code == 48 {
        return Some(if modifiers.shift {
            Request::PreviousTab
        } else {
            Request::NextTab
        });
    }
    if !modifiers.command || modifiers.control || modifiers.option {
        return (key_code == 53 && !modifiers.shift).then_some(Request::Hide);
    }
    // ⌘⏎。**綴りではなく key_code で見る**——`charactersIgnoringModifiers`
    // が返す制御文字は環境で揺れる。36 は Return の物理位置。
    if key_code == 36 && !modifiers.shift {
        return Some(Request::FocusTerminal);
    }
    Some(match key.as_str() {
        "b" => Request::Hide,
        "h" => Request::FocusCenter,
        // ⌘L は頁を見ている間はアドレス欄。右の一覧へ渡すのは頁が出ていないとき
        // だけで、その分岐は器の側が持つ。
        "l" => Request::FocusAddress,
        // ⇧⌘R は窓ごと起こし直す。**⌘R より先に見る**——後ろに置くと
        // shift 付きでも読み直しに食われる。
        "r" if modifiers.shift => Request::Restart,
        "r" => Request::Reload,
        "w" => Request::CloseTab,
        // ⇧⌘T は端末の側と同じく閉じた Claude タブを戻す。
        "t" if modifiers.shift => Request::Reopen,
        "t" => Request::NewBlank,
        // 頁の上でも器のキーを同じ指で押せるように。頁に渡すと WebView が黙って捨てる。
        "j" => Request::NewShell,
        "," | "、" => Request::Settings,
        "/" => Request::Help,
        "d" if modifiers.shift => Request::Bookmark,
        _ if key.len() == 1 && matches!(key.as_bytes()[0], b'1'..=b'9') => {
            Request::SelectTab(key.parse::<usize>().ok()?.saturating_sub(1))
        }
        _ => return None,
    })
}

/// ⌘[ で戻る・⌘] で進む。
///
/// 戻る/進むは `Request` にしない。`lib.rs` の分岐を1つも増やさずに済み、
/// 押した先も「いま見ているタブ」1枚で閉じるため、その場で WebView へ流す。
#[cfg(target_os = "macos")]
fn native_history_step(key: &str, modifiers: NativeModifiers) -> Option<i32> {
    if !modifiers.command || modifiers.control || modifiers.option || modifiers.shift {
        return None;
    }
    match key {
        "[" => Some(-1),
        "]" => Some(1),
        _ => None,
    }
}

/// いま見ているタブの履歴を1枚だけ動かす。
#[cfg(target_os = "macos")]
fn step_history(step: i32) {
    MANAGER.with(|manager| {
        if let Ok(manager) = manager.try_borrow()
            && let Some(view) = manager.tabs.get(manager.active).and_then(Tab::live)
        {
            let _ = if step < 0 {
                view.go_back()
            } else {
                view.go_forward()
            };
        }
    });
}

#[cfg(target_os = "macos")]
fn visible_native_request(
    visible: bool,
    key: &str,
    key_code: u16,
    modifiers: NativeModifiers,
) -> Option<Request> {
    visible
        .then(|| native_request(key, key_code, modifiers))
        .flatten()
}

/// ⌘1–9 の宛先を決める**唯一の分岐点**。
///
/// ```text
///   新規タブを見ている → 栞の1〜9番を開く
///   それ以外 → 従来どおりブラウザタブを直接選択
/// ```
///
/// 裁定が返って割り当てを変えるときは、ここだけを差し替える。呼ぶ側は
/// 「拾ったキーは食う」だけを守り、振り先を知らない。
fn contextual_request(request: Request) -> Option<Request> {
    let Request::SelectTab(index) = request else {
        return Some(request);
    };
    match active_new_tab() {
        // 栞が足りない番号は何も起こさない。ここでタブ選択へ落とすと、
        // 同じ指の動きが栞とタブを行き来して読めなくなる。
        Some((id, nonce)) => bookmark_shortcut(index, id, nonce, &bookmarks()),
        None => Some(Request::SelectTab(index)),
    }
}

/// いま見ているタブが新規タブなら、その id と nonce。
fn active_new_tab() -> Option<(u64, u64)> {
    MANAGER.with(|manager| {
        let manager = manager.try_borrow().ok()?;
        let tab = manager.tabs.get(manager.active)?;
        tab.new_tab_nonce.map(|nonce| (tab.id, nonce))
    })
}

/// 番号から栞1件を引く(0始まりの index は ⌘1 が 0 番)。
fn bookmark_shortcut(index: usize, id: u64, nonce: u64, bookmarks: &[Bookmark]) -> Option<Request> {
    let mark = bookmarks.get(index)?;
    Some(Request::OpenBookmark {
        id,
        nonce,
        url: mark.url.clone(),
    })
}

/// WebViewがAppKitの第一レスポンダでも、アプリの操作キーだけは
/// DOMやiframeを経由せずmacOSのイベント列から拾う。
#[cfg(target_os = "macos")]
pub fn install_native_shortcuts() {
    NATIVE_SHORTCUT_MONITOR.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        let handler = RcBlock::new(|event_ptr: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKitがlocal monitorの呼び出し中だけ有効なNSEventを渡す。
            let event = unsafe { event_ptr.as_ref() };
            let (visible, active) = MANAGER.with(|manager| {
                let manager = manager.borrow();
                (manager.visible, manager.tabs.get(manager.active).map(|tab| tab.id))
            });
            let flags = event.modifierFlags();
            let modifiers = NativeModifiers {
                command: flags.contains(NSEventModifierFlags::Command),
                control: flags.contains(NSEventModifierFlags::Control),
                option: flags.contains(NSEventModifierFlags::Option),
                shift: flags.contains(NSEventModifierFlags::Shift),
            };
            let key = event
                .charactersIgnoringModifiers()
                .map(|value| value.to_string())
                .unwrap_or_default();
            if visible {
                note_key_down(active, asks_for_claude(&key, modifiers));
            }
            if visible && let Some(step) = native_history_step(&key, modifiers) {
                step_history(step);
                return std::ptr::null_mut();
            }
            if let Some(request) = visible_native_request(visible, &key, event.keyCode(), modifiers)
            {
                // 拾ったキーは必ず食う。振り先が無い回に素通しすると、
                // 同じキーを頁のJS(`KEYS`)がもう一度拾う。
                queue(contextual_request(request));
                std::ptr::null_mut()
            } else {
                event_ptr.as_ptr()
            }
        });
        // SAFETY: handlerは`RcBlock`として有効で、返り値は元のイベントかnullのみ。
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler)
        };
        *slot.borrow_mut() = monitor;
    });
}

#[cfg(not(target_os = "macos"))]
pub fn install_native_shortcuts() {}

/// ⇧T(精訳)か。頁の JS も同じ鍵を見るが、頁は偽れるので器の側でも数える。
#[cfg(target_os = "macos")]
fn asks_for_claude(key: &str, modifiers: NativeModifiers) -> bool {
    modifiers.shift
        && !modifiers.command
        && !modifiers.control
        && !modifiers.option
        && key.eq_ignore_ascii_case("t")
}

/// 利用者が頁の上で鍵を押したことを覚える。⇧T なら、いま見ているタブに精訳を許す。
fn note_key_down(active: Option<u64>, claude: bool) {
    if let Ok(mut at) = KEY_DOWN_AT.lock() {
        *at = Some(std::time::Instant::now());
    }
    if claude
        && let Some(id) = active
        && let Ok(mut arm) = CLAUDE_ARM.lock()
    {
        *arm = Some((id, CLAUDE_ARM_BATCHES));
    }
}

/// 押した直後なら 1 回だけ真。外の頁が開く新しいタブを、押した回数までに抑える。
fn take_key_down() -> bool {
    let Ok(mut at) = KEY_DOWN_AT.lock() else {
        return false;
    };
    let fresh = at.is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(1500));
    *at = None;
    fresh
}

/// このタブに ⇧T の精訳が残っていれば 1 束ぶん使う。
fn take_claude_arm(id: u64) -> bool {
    let Ok(mut arm) = CLAUDE_ARM.lock() else {
        return false;
    };
    match arm.as_mut() {
        Some((tab, left)) if *tab == id && *left > 0 => {
            *left -= 1;
            true
        }
        _ => false,
    }
}

/// タブが今載せている頁の実の URL(WKWebView が持つ値。頁の申告ではない)。
fn page_url(id: u64) -> Option<String> {
    MANAGER.with(|manager| {
        let manager = manager.try_borrow().ok()?;
        let view = manager.tabs.iter().find(|tab| tab.id == id)?.live()?;
        view.url().ok()
    })
}

/// 伝言が外の頁から来たか。
///
/// 器の頁は `about:blank` に組み立てた頁(新規タブ・md・資料)と `file://` だけで、
/// 器の仕掛け([`page_script`])は頁の主フレームにだけ載る。内とみなすのは
/// 「主フレームから・送り主の URL も今載せている頁の実の URL も器の頁」のときだけ。
///
/// 主フレームかを見る理由: 器の頁(手元の HTML・md)に埋め込まれた外の iframe は、
/// 自分の中に `about:blank` や `about:srcdoc` の枠を立てればそこから言える。
/// 送り主の URL だけでは器の頁と見分けがつかない(2026-09-26 に塞いだ穴)。
fn from_outside(sender: &str, main_frame: bool, page: Option<&str>) -> bool {
    let inside = |url: &str| {
        url == "about:blank" || url == "about:srcdoc" || url.starts_with("file:")
    };
    !(main_frame && inside(sender) && page.is_some_and(inside))
}

/// 頁から届いた伝言を1通受ける。`sender` は伝言を出したフレームの URL、`main_frame` は
/// それがタブの主フレームか(iframe ではないか)。
fn receive(tab: u64, body: &str, sender: &str, main_frame: bool) {
    // 器の仕掛けは主フレームにしか載らない(wry の初期スクリプトは主フレームだけ)ので、
    // iframe から来る伝言に正規のものは無い。題の書き換えも含めて全部捨てる
    // ——手元の頁に埋め込まれた広告の枠がタブの名前を偽れた(2026-09-26 に VM で実測)。
    if !main_frame {
        return;
    }
    // **送り主で仕分ける。**外の頁からは限られた伝言だけを受ける([`outside_request`])。
    let page = page_url(tab);
    let outside = from_outside(sender, main_frame, page.as_deref());
    // 原文で読むドメインの読み書きは頁と直接やり取りする。iced側の
    // 更新を待たせると、頁が翻訳を始めてしまう。
    if let Some(message) = translation_site_message(body) {
        // 外の頁が書き換えてよいのは自分のドメインの選択だけ。
        if outside
            && let SiteMessage::Set { host, .. } = &message
            && page.as_deref().map(host_of).as_deref().and_then(normalized_host).as_ref()
                != Some(host)
        {
            return;
        }
        answer_translation_site(tab, &message);
        return;
    }
    let request = match parse(tab, body) {
        // 行き先は頁の申告ではなく WKWebView の実の URL で書く。
        Some(Request::Location { id, .. }) => page.clone().map(|url| Request::Location { id, url }),
        other => other,
    };
    queue(if outside { request.and_then(outside_request) } else { request });
}

/// 頁からの伝言の受け口(`window.webkit.messageHandlers.ipc`)。タブごとに1つ。
///
/// wry の受け口と違い、伝言を出したフレームが主フレームかを WebKit(`WKFrameInfo`)に
/// 訊いてから [`receive`] へ渡す。`file://` の頁からの伝言も落とさない(wry は送り主の
/// URL を `http::Uri` に組めないと黙って捨てていた)。
mod ipc {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::NSString;
    use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController};

    pub(super) struct Ivars {
        tab: u64,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = Ivars]
        pub(super) struct IpcReceiver;

        unsafe impl NSObjectProtocol for IpcReceiver {}

        unsafe impl WKScriptMessageHandler for IpcReceiver {
            #[unsafe(method(userContentController:didReceiveScriptMessage:))]
            fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
                // SAFETY: WebKit が窓の糸で渡す伝言。本文と送り主の枠を読むだけ。
                let (body, sender, main_frame) = unsafe {
                    let Ok(body) = message.body().downcast::<NSString>() else {
                        return;
                    };
                    let frame = message.frameInfo();
                    let sender = frame
                        .request()
                        .URL()
                        .and_then(|url| url.absoluteString())
                        .map(|url| url.to_string())
                        .unwrap_or_default();
                    (body.to_string(), sender, frame.isMainFrame())
                };
                super::receive(self.ivars().tab, &body, &sender, main_frame);
            }
        }
    );

    /// タブの WebView に伝言の受け口を据える。頁の `window.ipc.postMessage` はここへ届く。
    pub(super) fn install(webview: &wry::WebView, tab: u64) {
        use wry::WebViewExtMacOS;

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let receiver = mtm.alloc::<IpcReceiver>().set_ivars(Ivars { tab });
        // SAFETY: NSObject の init。
        let receiver: Retained<IpcReceiver> = unsafe { msg_send![super(receiver), init] };
        // SAFETY: 窓の糸。受け口は WebView の設定が保持し、WebView と一緒に消える。
        unsafe {
            webview
                .webview()
                .configuration()
                .userContentController()
                .addScriptMessageHandler_name(
                    ProtocolObject::from_ref(&*receiver),
                    &NSString::from_str("ipc"),
                );
        }
    }
}

/// 外の頁から受けてよい伝言だけを通す。
///
/// 外の頁(と、その中の広告の枠)は `window.ipc` で何でも言える。題・行き先の知らせ・
/// 自動翻訳・払いの戻る進むは受け、精訳は ⇧T を押したタブだけ、新しいタブは鍵を
/// 押した直後の web の頁だけ。手元のファイル・再起動・タブ操作・栞は器の頁と
/// native の鍵からだけにする。
fn outside_request(request: Request) -> Option<Request> {
    match request {
        Request::Title { .. } | Request::Location { .. } | Request::History(_) => Some(request),
        Request::Translate { id, claude, .. } => (!claude || take_claude_arm(id)).then_some(request),
        Request::NewTab(ref url) => (is_web_url(url) && take_key_down()).then_some(request),
        _ => None,
    }
}

/// **頁から器へ物を言う唯一の口。**頁に載せる仕掛けは全部これを通す。
///
/// 口は主フレームの `window.ipc`(受けるのは [`ipc::install`] が据えた受け口)。
/// 器は伝言を出したフレームが主フレームかを見て仕分ける([`from_outside`])ので、
/// iframe から投げた伝言は外の頁のものとして扱われる。
///
/// 以前は `file://` の頁から `about:blank` の隠しフレームを立ててそこから投げていた。
/// wry の受け口は送り主の URL を `http::Uri` に組めないと伝言を捨て、`file:///…`
/// (宿の名が空)は組めなかったため(2026-09-06 実測)。受け口を自前にしたので、
/// 回り道は要らない——隠しフレームから投げると、今は外の伝言として弾かれる。
const SAY: &str = r#"
;(function () {
  if (window.__cockpitSay) { return; }
  window.__cockpitSay = function (value) {
    try { window.ipc.postMessage(value); return true; } catch (_) { return false; }
  };
})();
"#;

const KEYS: &str = r#"
;(function () {
  if (window.top !== window) { return; }
  if (window.__cockpitKeys) { return; }
  window.__cockpitKeys = true;
  function say(value) { try { window.__cockpitSay(value); } catch (_) {} }
  function typing() {
    var a = document.activeElement;
    if (!a) { return false; }
    var tag = (a.tagName || '').toLowerCase();
    return a.isContentEditable || tag === 'input' || tag === 'textarea' || tag === 'select';
  }
  // j/k の一段。以前は scrollBy で 56px を瞬間移動させていたので、押すたび
  // 頁が飛んで読んでいた行を見失った。目標だけを積み、実際の位置は毎フレーム
  // 残りの距離を一定割合ずつ詰めて寄せる。連打すると目標が積み上がるので、
  // 速く押すほど速く流れて、離せば自然に止まる。
  var STEP = 120;
  window.__cockpitScroll = (function () {
    // **位置は経過時間だけで決める。コマの数では決めない。**
    // 以前はコマごとに「残りの一定割合」を詰めていたが、その作りはコマが落ちた
    // ぶんの距離を置き去りにする。画像の多い頁では実効 6.7fps まで落ちるので
    // (2026-08-30 実測・埋め込み22枚のドラフト)、遅れが溜まってから一度に
    // 吐き出し、1コマで 463px 飛んでいた——これがガクつきの正体。時間の関数に
    // すれば、何枚落ちても「その時刻にあるべき位置」へ置ける。
    var target = null, boxEl = null, raf = 0, origin = 0, t0 = 0, stalls = 0;
    // 残りが 1/e になるまでの時間(ms)。押し味はここで決まる。
    var TAU = 130;
    function stop() {
      target = null; boxEl = null; stalls = 0;
      if (raf) { cancelAnimationFrame(raf); raf = 0; }
    }
    function top(el) {
      return el ? el.scrollTop : (window.scrollY || document.documentElement.scrollTop || 0);
    }
    function limit(el) {
      if (el) return Math.max(0, el.scrollHeight - el.clientHeight);
      var doc = document.documentElement, body = document.body;
      var height = Math.max(doc ? doc.scrollHeight : 0, body ? body.scrollHeight : 0);
      return Math.max(0, height - window.innerHeight);
    }
    function put(el, value) {
      // サイト側の scroll-behavior:smooth に乗ると二重に緩んで、かえって
      // 遅れて着く。ここでは必ず即時に置き、緩みはこちらの計算だけで作る。
      if (el) el.scrollTo({top: value, left: el.scrollLeft, behavior: 'instant'});
      else window.scrollTo({top: value, left: window.scrollX, behavior: 'instant'});
    }
    function scrollable(from) {
      for (var p = from; p && p !== document.body && p !== document.documentElement; p = p.parentElement) {
        var style = getComputedStyle(p);
        if (!/(auto|scroll|overlay)/.test(style.overflowY)) continue;
        if (p.scrollHeight - p.clientHeight > 4) return p;
      }
      return null;
    }
    // 窓そのものに動く余地が無い頁(内側のdivが流れる作り)では、画面の
    // 真ん中にある流せる箱を選ぶ。余地があるなら今までどおり窓を動かす。
    function surface() {
      if (limit(null) > 4) return null;
      var at = document.elementFromPoint(window.innerWidth / 2, window.innerHeight / 2);
      return at ? scrollable(at) : null;
    }
    function step(now) {
      raf = 0;
      if (target === null) return;
      var here = top(boxEl);
      // 押した時刻からの経過で、いまあるべき位置を出す。
      var want = target - (target - origin) * Math.exp(-(now - t0) / TAU);
      if (Math.abs(target - want) < 0.6) { put(boxEl, target); stop(); return; }
      put(boxEl, want);
      // 置いても動かないなら(固定・スナップ・別の主が握っている)追わない。
      // **1コマの空振りでは降りない**——重い頁では丸めや遅れで普通に起きるので、
      // 1コマで降りると読んでいる途中で追従が止まり、次の押しまで固まって見える。
      if (Math.abs(top(boxEl) - here) < 0.4 && Math.abs(want - here) > 1) {
        if (++stalls >= 4) { stop(); return; }
      } else { stalls = 0; }
      raf = requestAnimationFrame(step);
    }
    function by(amount) {
      if (target === null) boxEl = surface();
      var here = top(boxEl);
      var base = target === null ? here : target;
      target = Math.min(limit(boxEl), Math.max(0, base + amount));
      // 起点は「いまの実位置」、時計はここから引き直す。目標だけが積み上がるので、
      // 速く押すほど速く流れて、離せば自然に止まる——ここは前と同じ手触り。
      origin = here; t0 = performance.now(); stalls = 0;
      if (!raf) raf = requestAnimationFrame(step);
    }
    return {by: by, stop: stop, step: STEP};
  })();
  // 利用者が別の手で動かし始めたら、こちらの目標は捨てる。競り合うと震える。
  window.addEventListener('wheel', function () { window.__cockpitScroll.stop(); }, {passive: true});
  window.addEventListener('mousedown', function () { window.__cockpitScroll.stop(); }, {passive: true});
  window.addEventListener('keydown', function (event) {
    var key = event.key;
    var lower = key.toLowerCase();
    var message = null;
    if (window.__cockpitHints && window.__cockpitHints.feed(event)) {
      event.preventDefault(); event.stopImmediatePropagation(); return;
    }
    if (key === 'Escape' && typing()) {
      document.activeElement.blur(); event.preventDefault(); return;
    }
    if (key === 'Escape' || (event.metaKey && lower === 'b')) { message = 'hide'; }
    else if (event.metaKey && lower === 'h') { message = 'focus:center'; }
    else if (event.metaKey && lower === 'l') { message = 'focus:address'; }
    else if (event.metaKey && lower === 'w') { message = 'tab:close'; }
    else if (event.metaKey && lower === 't' && !event.shiftKey) { message = 'tab:new'; }
    else if (event.metaKey && key === '/') { message = 'help'; }
    else if (event.metaKey && lower === 'r') { message = event.shiftKey ? 'restart' : 'reload'; }
    else if (event.metaKey && !event.shiftKey && key === 'Enter') { message = 'focus:terminal'; }
    else if (event.metaKey && lower === 'd') { message = 'bookmark'; }
    else if (event.metaKey && /^[1-9]$/.test(key)) { message = 'tab:index:' + key; }
    else if (!typing() && !event.metaKey && !event.ctrlKey && !event.altKey && key === 'j') {
      if (window.__cockpitNewTab && window.__cockpitNewTab.move(1)) {
        event.preventDefault(); event.stopImmediatePropagation(); return;
      }
      window.__cockpitScroll.by(STEP); event.preventDefault(); return;
    } else if (!typing() && !event.metaKey && !event.ctrlKey && !event.altKey && key === 'k') {
      if (window.__cockpitNewTab && window.__cockpitNewTab.move(-1)) {
        event.preventDefault(); event.stopImmediatePropagation(); return;
      }
      window.__cockpitScroll.by(-STEP); event.preventDefault(); return;
    } else if (!typing() && !event.metaKey && !event.ctrlKey && !event.altKey && key === 'h') {
      history.back(); event.preventDefault(); event.stopImmediatePropagation(); return;
    } else if (!typing() && !event.metaKey && !event.ctrlKey && !event.altKey && key === 'l') {
      history.forward(); event.preventDefault(); event.stopImmediatePropagation(); return;
    }
    if (message) {
      event.preventDefault(); event.stopPropagation(); say(message);
    }
  }, true);
  function identify() {
    say('title:' + (document.title || ''));
    say('location:' + String(location.href || ''));
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', identify);
  else identify();
  addEventListener('popstate', identify);
  addEventListener('hashchange', identify);
})();
"#;

// WebKit の rubber-band 中も頁自身に不透明な地面を持たせる。ただし透明な頁を
// 無条件にアプリの濃色へ置くと、サイト既定の黒文字が読めなくなる。サイトが
// 背景を持つ場合は触らず、透明な場合だけ実際の本文色から紙の明暗を選ぶ。
/// 二本指の横払いで履歴を辿る。
///
/// **WKWebView の作り付けの身振りは当てにしない。** `allowsBackForwardNavigation-
/// Gestures` を立て、頁の `overscroll-behavior-x` も `auto` へ戻したのに動かなかった。
/// 子ビューとして貼った
/// WKWebView では拾われないらしい。そこで横の転がりを自分で数え、native の
/// `goBack`/`goForward` を叩く——‹ › の釦と同じ口なので、動きは1つに揃う。
///
/// 横に流せる箱(広い表・コード塊)の上では、その箱が端に着くまで払いを渡さない。
/// 渡してしまうと、表を横に見ようとしただけで頁が戻る。
const SWIPE: &str = r#"
;(function () {
  if (window.top !== window) { return; }
  if (window.__cockpitSwipe) { return; }
  window.__cockpitSwipe = true;
  function say(value) { try { window.__cockpitSay(value); } catch (_) {} }
  // 払いと認める横の量。小さいと表を撫でただけで戻る。
  var THRESHOLD = 90;
  var sum = 0, last = 0, locked = 0;
  function scrollableX(from) {
    for (var p = from; p && p !== document.body && p !== document.documentElement; p = p.parentElement) {
      var style = getComputedStyle(p);
      if (!/(auto|scroll|overlay)/.test(style.overflowX)) continue;
      if (p.scrollWidth - p.clientWidth > 4) return p;
    }
    var doc = document.scrollingElement || document.documentElement;
    return (doc && doc.scrollWidth - doc.clientWidth > 4) ? doc : null;
  }
  addEventListener('wheel', function (event) {
    var now = event.timeStamp;
    // 指を離せば数えは切れる。続きの払いと繋げない。
    if (now - last > 400) { sum = 0; }
    last = now;
    if (Math.abs(event.deltaX) <= Math.abs(event.deltaY)) { sum = 0; return; }
    var box = scrollableX(event.target);
    if (box) {
      var atLeft = box.scrollLeft <= 0;
      var atRight = box.scrollLeft >= box.scrollWidth - box.clientWidth - 1;
      var goingLeft = event.deltaX < 0;
      if (!(goingLeft && atLeft) && !(!goingLeft && atRight)) { sum = 0; return; }
    }
    if (now < locked) { return; }
    sum += event.deltaX;
    if (sum <= -THRESHOLD) { locked = now + 900; sum = 0; say('history:-1'); }
    else if (sum >= THRESHOLD) { locked = now + 900; sum = 0; say('history:1'); }
  }, {passive: true});
})();
"#;

const PAGE_SURFACE: &str = r#"
;(function () {
  if (window.top !== window) { return; }
  // 縦だけ止める。**横まで止めると二本指の「戻る/進む」が消える**
  // ——WebKit は overscroll-behavior-x が none の頁でスワイプ移動を切る。
  // ゴム跳ねの下地が見えるのは
  // 縦の話なので、止めるのも縦だけでよい。
  var css = 'html{min-height:100%;}' +
    'html,body{overscroll-behavior-y:none !important;overscroll-behavior-x:auto !important;}' +
    'body{min-height:100%;}';
  var painted = false, timer = null, observer = null;
  function transparent(value) {
    if (!value || value === 'transparent') return true;
    var match = value.match(/^rgba?\(([^)]+)\)$/i);
    if (!match) return false;
    var parts = match[1].split(',').map(function (part) { return parseFloat(part); });
    return parts.length > 3 && parts[3] === 0;
  }
  function hasSurface(style) {
    return !transparent(style.backgroundColor) || style.backgroundImage !== 'none';
  }
  function brightness(value) {
    var match = value && value.match(/^rgba?\(([^)]+)\)$/i);
    if (!match) return 0;
    var rgb = match[1].split(',').slice(0, 3).map(function (part) { return parseFloat(part); });
    return (rgb[0] * 299 + rgb[1] * 587 + rgb[2] * 114) / 1000;
  }
  function textProbe(root) {
    // selectorをカンマで並べても返る順序はDOM順。navの薄い文字を先に拾うと
    // 黒い本文なのに暗い紙面を選ぶため、記事本文を段階的に優先する。
    var selectors = ['article p','main p','article','main','p','section','h1','li'];
    for (var i = 0; i < selectors.length; i++) {
      var candidate = root.querySelector(selectors[i]);
      if (candidate && (candidate.textContent || '').trim()) return candidate;
    }
    return document.body || root;
  }
  // 地の色に触らない着付け——ゴム跳ねの止めと自前の様式だけ。
  function dress(root) {
    root.style.setProperty('overscroll-behavior', 'none', 'important');
    var style = document.getElementById('__cockpitSurface');
    if (!style) {
      style = document.createElement('style');
      style.id = '__cockpitSurface';
      style.textContent = css;
      (root.head || root).appendChild(style);
    }
    if (document.body) document.body.style.setProperty('overscroll-behavior', 'none', 'important');
  }
  function apply() {
    var root = document.documentElement;
    if (!root) { return; }
    // 前回こちらが置いた色を外してから、サイト自身の背景を測る。同じJSタスク内
    // なので中間状態は描画されず、SPAが後から本来の背景を足した場合も尊重できる。
    if (painted) {
      root.style.removeProperty('background-color');
      if (document.body) document.body.style.removeProperty('background-color');
      painted = false;
    }
    var rootStyle = getComputedStyle(root);
    var bodyStyle = document.body ? getComputedStyle(document.body) : null;
    if (!hasSurface(rootStyle) && (!bodyStyle || !hasSurface(bodyStyle))) {
      var ink = getComputedStyle(textProbe(root)).color;
      var surface = brightness(ink) < 150 ? '#f7f5ef' : '#303446';
      // サイト側が透明色を !important で指定していても、選んだ紙面色は確実に描く。
      // 次の apply 冒頭で外すため、サイトが後から本来の背景を持った場合は邪魔しない。
      root.style.setProperty('background-color', surface, 'important');
      // 透明な WKWebView では root のキャンバス色が背面へ抜ける頁がある。
      // body にも同じ紙面を置き、本文の下まで確実に不透明にする。
      if (document.body) document.body.style.setProperty('background-color', surface, 'important');
      painted = true;
    }
    dress(root);
  }
  function schedule() {
    clearTimeout(timer); timer = setTimeout(apply, 60);
  }
  function observe() {
    if (observer || !document.documentElement) return;
    observer = new MutationObserver(schedule);
    observer.observe(document.documentElement, {childList:true,subtree:true});
  }
  // **最初の一回は待たずに塗る。**60ms の間を置くと、その間に最初の描画が
  // 来て、地の色が一度違う色で出てから直る。
  function start() { observe(); apply(); }
  // **頁が始まった瞬間には地の色を決めない。**この時点では <html> しか無く
  // 様式はまだ読まれていないので、どの頁も「背景が無く字が黒い」と見えて
  // 明るい紙を敷いてしまう——自分で描く暗い新規タブまで、最初の数コマだけ
  // 紙で光っていた。紙を敷くかは様式が入った DOMContentLoaded で決める。
  if (document.documentElement) dress(document.documentElement);
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start);
  else start();
  addEventListener('load', start);
})();
"#;

// Vimium 風のリンクヒント。f は入力欄では奪わず、表示中のリンクだけを短い
// 英字ラベルで選ぶ。通常のリンクは同じ頁で遷移し、Shift または target=_blank
// は既存の tab:open IPC へ渡して新しいタブにする。
const HINTS: &str = r#"
;(function () {
  if (window.top !== window || window.__cockpitHints) { return; }
  var CHARS = 'asdfghjklqwertyuiopzxcvbnm';
  var SELECTOR = 'a[href],button,summary,select,textarea,' +
    'input:not([type=hidden]):not([disabled]),' +
    '[role="button"],[role="link"],[role="tab"],[role="menuitem"],[role="checkbox"],' +
    '[onclick],[tabindex]:not([tabindex="-1"])';
  var hints = [], buffer = '', layer = null, pending = 0;
  function typing() {
    var a = document.activeElement;
    if (!a) { return false; }
    var tag = (a.tagName || '').toLowerCase();
    return a.isContentEditable || tag === 'input' || tag === 'textarea' || tag === 'select';
  }
  function clear() {
    if (pending) { cancelAnimationFrame(pending); pending = 0; }
    if (layer) { layer.remove(); layer = null; }
    hints = []; buffer = '';
  }
  // 画面に実際に出ている部分だけを矩形で返す。祖先の overflow に切られた分も
  // 差し引く——見えていない所へ札を出すと、押した先が狙いから外れる。
  function box(el) {
    var rect = el.getBoundingClientRect();
    if (rect.width < 4 || rect.height < 4) return null;
    var top = rect.top, left = rect.left, bottom = rect.bottom, right = rect.right;
    for (var p = el.parentElement; p && p !== document.documentElement; p = p.parentElement) {
      var ps = getComputedStyle(p);
      if (ps.overflow === 'visible' && ps.overflowX === 'visible' && ps.overflowY === 'visible') continue;
      var pr = p.getBoundingClientRect();
      if (pr.width < 1 || pr.height < 1) continue;
      top = Math.max(top, pr.top); left = Math.max(left, pr.left);
      bottom = Math.min(bottom, pr.bottom); right = Math.min(right, pr.right);
    }
    top = Math.max(top, 0); left = Math.max(left, 0);
    bottom = Math.min(bottom, innerHeight); right = Math.min(right, innerWidth);
    if (bottom - top < 4 || right - left < 4) return null;
    var style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility !== 'visible') return null;
    if (parseFloat(style.opacity || '1') < 0.1) return null;
    if (style.pointerEvents === 'none') return null;
    return {top: top, left: left, bottom: bottom, right: right};
  }
  // 重なった別の面に隠れていないか。見えている矩形の3点を突いて、
  // 1点でも自分に当たれば前面。隠れた要素に札を出すと狙いが外れる。
  function frontmost(el, b) {
    var wide = Math.min(10, (b.right - b.left) / 2), tall = Math.min(6, (b.bottom - b.top) / 2);
    var xs = [b.left + wide, (b.left + b.right) / 2, b.right - wide];
    var ys = [(b.top + b.bottom) / 2, b.top + tall, b.bottom - tall];
    for (var i = 0; i < 3; i++) {
      var at = document.elementFromPoint(xs[i], ys[i]);
      if (at && (at === el || el.contains(at) || at.contains(el))) return true;
    }
    return false;
  }
  function labels(count) {
    var out = [], i, j;
    if (count <= CHARS.length) {
      for (i = 0; i < count; i++) out.push(CHARS[i]);
      return out;
    }
    for (i = 0; i < CHARS.length && out.length < count; i++)
      for (j = 0; j < CHARS.length && out.length < count; j++)
        out.push(CHARS[i] + CHARS[j]);
    return out;
  }
  function paint() {
    hints.forEach(function (hint) {
      var matched = hint.name.indexOf(buffer) === 0;
      hint.badge.style.display = matched ? 'block' : 'none';
      if (!matched) return;
      hint.head.textContent = hint.name.slice(0, buffer.length);
      hint.tail.textContent = hint.name.slice(buffer.length);
    });
  }
  function show() {
    clear();
    var targets = [];
    var nodes = Array.prototype.slice.call(document.querySelectorAll(SELECTOR));
    for (var i = 0; i < nodes.length; i++) {
      var el = nodes[i], b = box(el);
      if (!b || !frontmost(el, b)) continue;
      // 入れ子(aの中のbutton等)で同じ場所に札が二重に出ると、どちらを
      // 指したのか分からなくなる。ほぼ同じ矩形は先に見つけた方だけ残す。
      var dup = false;
      for (var j = 0; j < targets.length; j++) {
        var s = targets[j].box;
        if (Math.abs(s.left - b.left) < 6 && Math.abs(s.top - b.top) < 6 &&
            Math.abs(s.right - b.right) < 6 && Math.abs(s.bottom - b.bottom) < 6) { dup = true; break; }
      }
      if (!dup) targets.push({el: el, box: b});
    }
    if (!targets.length) return;
    // 札はDOM順ではなく画面の並び順に振る。目で追う順と一致しないと、
    // 上から3つ目のリンクに遠い札が付いて選び間違える。
    targets.sort(function (a, b) {
      var dy = a.box.top - b.box.top;
      if (Math.abs(dy) > 8) return dy;
      return a.box.left - b.box.left;
    });
    var names = labels(targets.length);
    layer = document.createElement('div');
    layer.style.cssText = 'position:fixed;inset:0;z-index:2147483646;pointer-events:none;';
    var placed = [];
    targets.forEach(function (target, index) {
      var name = names[index];
      var w = 9 * name.length + 9, h = 16;
      var x = target.box.left, y = target.box.top;
      // 置いた札と重なったら逃がす。札同士が重なると、どれがどの行のものか
      // 読めなくなり、結局は狙いが外れる。
      for (var tries = 0; tries < 8; tries++) {
        var hit = false;
        for (var k = 0; k < placed.length; k++) {
          var p = placed[k];
          if (x < p.x + p.w && x + w > p.x && y < p.y + p.h && y + h > p.y) { hit = true; break; }
        }
        if (!hit) break;
        y += h + 1;
        if (y + h > innerHeight) { y = target.box.top; x += w + 1; }
      }
      x = Math.min(Math.max(0, x), Math.max(0, innerWidth - w));
      y = Math.min(Math.max(0, y), Math.max(0, innerHeight - h));
      placed.push({x: x, y: y, w: w, h: h});
      var badge = document.createElement('span');
      badge.style.cssText = 'position:fixed;left:' + x + 'px;top:' + y + 'px;' +
        'padding:1px 4px;border-radius:4px;background:#f2c94c;box-shadow:0 1px 3px rgba(0,0,0,.4);' +
        'color:#303446;font:bold 12px/1.2 -apple-system,sans-serif;letter-spacing:.5px;';
      var head = document.createElement('i'), tail = document.createElement('i');
      head.style.cssText = 'font-style:normal;opacity:.35;';
      tail.style.cssText = 'font-style:normal;';
      badge.appendChild(head); badge.appendChild(tail);
      layer.appendChild(badge);
      hints.push({name: name, element: target.el, badge: badge, head: head, tail: tail});
    });
    document.documentElement.appendChild(layer);
    paint();
  }
  // 押す。href を持つものは今までどおり遷移させ、持たないものは実座標へ
  // マウス列を合成する。el.click() だけでは pointer 系しか見ていない実装に
  // 当たらず、押したつもりで何も起きない。
  function activate(el, shift) {
    var tag = (el.tagName || '').toLowerCase();
    var type = (el.getAttribute('type') || '').toLowerCase();
    if (tag === 'textarea' || tag === 'select' || el.isContentEditable ||
        (tag === 'input' && type !== 'button' && type !== 'submit' &&
         type !== 'checkbox' && type !== 'radio' && type !== 'reset')) {
      try { el.focus({preventScroll: true}); } catch (_) { el.focus(); }
      return true;
    }
    var attr = el.getAttribute('href');
    if (attr && !/^javascript:/i.test(attr)) {
      var href = el.href || attr;
      var blank = (el.getAttribute('target') || '').toLowerCase() === '_blank';
      if (shift || blank) {
        try { window.__cockpitSay('tab:open:' + href); } catch (_) {}
      } else {
        location.href = href;
      }
      return true;
    }
    var rect = el.getBoundingClientRect();
    var cx = rect.left + rect.width / 2, cy = rect.top + rect.height / 2;
    try { el.focus({preventScroll: true}); } catch (_) {}
    ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click'].forEach(function (kind) {
      var pointer = kind.indexOf('pointer') === 0 && typeof PointerEvent === 'function';
      var init = {bubbles: true, cancelable: true, view: window, button: 0, buttons: 1,
        clientX: cx, clientY: cy, detail: kind === 'click' ? 1 : 0};
      if (pointer) { init.pointerId = 1; init.pointerType = 'mouse'; init.isPrimary = true; }
      el.dispatchEvent(new (pointer ? PointerEvent : MouseEvent)(kind, init));
    });
    return true;
  }
  function pick(name, shift) {
    var hit = null;
    for (var i = 0; i < hints.length; i++) if (hints[i].name === name) { hit = hints[i]; break; }
    if (!hit) return false;
    var element = hit.element;
    clear();
    return activate(element, shift);
  }
  function feed(event) {
    if (!hints.length) return false;
    if (event.metaKey || event.ctrlKey || event.altKey) { clear(); return false; }
    if (event.key === 'Escape') { clear(); return true; }
    if (event.key === 'Backspace') {
      if (!buffer) { clear(); return true; }
      buffer = buffer.slice(0, -1); paint(); return true;
    }
    var key = event.key.toLowerCase();
    if (key.length !== 1 || CHARS.indexOf(key) < 0) { clear(); return false; }
    var next = buffer + key;
    var possible = hints.some(function (hint) { return hint.name.indexOf(next) === 0; });
    if (!possible) { clear(); return true; }
    buffer = next;
    if (hints.some(function (hint) { return hint.name === buffer; })) pick(buffer, event.shiftKey);
    else paint();
    return true;
  }
  window.__cockpitHints = { show: show, clear: clear, feed: feed };
  window.addEventListener('keydown', function (event) {
    if (typing() || event.metaKey || event.ctrlKey || event.altKey || hints.length) return;
    if (event.key === 'f' && !event.shiftKey) {
      // j/k で流している最中に測ると、止まる前の座標で札を置いてしまう。
      // まず流れを止め、1フレーム待ってレイアウトが落ち着いてから測る。
      if (window.__cockpitScroll) window.__cockpitScroll.stop();
      requestAnimationFrame(show);
      event.preventDefault(); event.stopPropagation();
    }
  }, true);
  // 札を出したまま頁が動いたら、消さずに測り直す。ただし打ちかけの間は
  // 割り当てを動かさない——同じ札が別のものを指したら押し間違える。
  window.addEventListener('scroll', function () {
    if (!hints.length || pending || buffer) return;
    pending = requestAnimationFrame(function () { pending = 0; if (hints.length && !buffer) show(); });
  }, {passive: true});
})();
"#;

// WKWebViewへ頁が描かれる前から動く、対象を絞った広告除去。
// `class*=ad` のような広すぎる規則は本文まで消すため使わず、広告SDKが明示する
// slot属性と既知の配信hostだけを対象にする。MutationObserverで後挿入にも追随する。
const AD_BLOCK: &str = r#"
;(function () {
  if (window.top !== window || window.__cockpitAdBlock) return;
  window.__cockpitAdBlock = true;
  var hosts = [
    'doubleclick.net','googlesyndication.com','googleadservices.com',
    'amazon-adsystem.com','adnxs.com','criteo.com','criteo.net',
    'taboola.com','outbrain.com'
  ];
  var selectors = [
    'ins.adsbygoogle','[data-ad-slot]','[data-ad-client]','[id^="google_ads_"]',
    'iframe[id^="google_ads_"]','iframe[name^="google_ads_"]',
    '[data-testid="ad"]','[aria-label="Advertisement"]'
  ];
  function adUrl(value) {
    if (!value) return false;
    try {
      var host = new URL(value, document.baseURI).hostname.toLowerCase();
      return hosts.some(function (blocked) {
        return host === blocked || host.endsWith('.' + blocked);
      });
    } catch (_) { return false; }
  }
  function resourceUrl(node) {
    if (!node || !node.tagName) return '';
    var tag = node.tagName.toLowerCase();
    if (['script','iframe','img','source','video','audio','link','ins'].indexOf(tag) < 0) return '';
    return node.getAttribute('src') || node.getAttribute('href') || node.getAttribute('data-src');
  }
  function discard(node) {
    if (!node || node.nodeType !== 1) return;
    if ((node.matches && node.matches(selectors.join(','))) ||
        adUrl(resourceUrl(node))) {
      node.remove(); return;
    }
    if (!node.querySelectorAll) return;
    node.querySelectorAll(selectors.join(',')).forEach(function (item) { item.remove(); });
    node.querySelectorAll('script[src],iframe[src],img[src],source[src],video[src],audio[src],link[href],ins[data-src]').forEach(function (item) {
      if (adUrl(resourceUrl(item))) item.remove();
    });
  }
  function start() {
    discard(document.documentElement);
    new MutationObserver(function (records) {
      records.forEach(function (record) {
        Array.prototype.forEach.call(record.addedNodes || [], discard);
      });
    }).observe(document.documentElement, {childList:true,subtree:true,attributes:true,attributeFilter:['src','href','data-src']});
  }
  if (document.documentElement) start();
  else document.addEventListener('DOMContentLoaded', start, {once:true});
})();
"#;

/// wry へ渡せる綴りか。
///
/// wry は `NSURL URLWithString` の答えを確かめずに unwrap するので、`http://exa mple.com`
/// のような読めない綴りを渡すと窓ごと落ちる。渡す前にここで同じ関数に訊く。
fn loadable(url: &str) -> bool {
    if url.is_empty() || url.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        objc2_foundation::NSURL::URLWithString(&objc2_foundation::NSString::from_str(url)).is_some()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// 頁を差し替える。読めない綴りは wry へ渡さない([`loadable`])。
fn load(view: &WebView, url: &str) -> wry::Result<()> {
    if !loadable(url) {
        return Err(wry::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            tr!(format!("not an address: {url}"), format!("行き先として読めない: {url}")),
        )));
    }
    view.load_url(url)
}

/// 出荷時から原文で読むドメイン。実行時に `t` で覆せる。
const DEFAULT_ORIGINAL_HOSTS: &[&str] = &[
    "github.com",
    "gitlab.com",
    "crates.io",
    "docs.rs",
    "pkg.go.dev",
    "npmjs.com",
    "pypi.org",
];

/// 束に入れて出す選択の既定(`assets/browser-translation.json`)。
const EMBEDDED_TRANSLATION_SITES: &str = include_str!("../assets/browser-translation.json");

/// 実行時に選んだドメインだけを持つ(true = 原文で表示する)。
/// 既定リストはここへ写さない——定数を後から書き換えても効くようにするため。
static TRANSLATION_SITES: LazyLock<Mutex<BTreeMap<String, bool>>> =
    LazyLock::new(|| Mutex::new(load_translation_sites()));

#[derive(Clone, Debug, PartialEq, Eq)]
enum SiteMessage {
    /// 頁を開いた側からの問い合わせ。埋め込み済みの一覧より新しい判定を返す。
    Ask(String),
    /// `t` で選ばれた表示。true = このドメインは今後原文で。
    Set { host: String, original: bool },
}

fn normalized_host(raw: &str) -> Option<String> {
    let host = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.len() > 253 {
        return None;
    }
    host.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
        .then_some(host)
}

/// このホストを原文で見せるか。JS側の `original()` と同じ規則を持つ。
///
/// 一致は「完全一致」か「`.` を挟んだ後方一致」だけ。`github.com` は
/// `sub.github.com` を含み、`github.com.evil.jp` は含まない。複数に当たれば
/// 長い(=細かい)方が勝つので、既定の `github.com` を原文にしたまま
/// `gist.github.com` だけ翻訳へ戻せる。
fn shows_original(sites: &BTreeMap<String, bool>, host: &str) -> bool {
    let Some(host) = normalized_host(host) else {
        return false;
    };
    let mut best: Option<(usize, bool)> = None;
    for (entry, original) in sites {
        let under = host == *entry
            || host
                .strip_suffix(entry.as_str())
                .is_some_and(|head| head.ends_with('.'));
        if under && best.is_none_or(|(len, _)| entry.len() > len) {
            best = Some((entry.len(), *original));
        }
    }
    best.is_some_and(|(_, original)| original)
}

fn parse_translation_sites(source: &str) -> Option<BTreeMap<String, bool>> {
    let root: serde_json::Value = serde_json::from_str(source).ok()?;
    let object = root.get("sites")?.as_object()?;
    let mut values = BTreeMap::new();
    for (host, value) in object {
        let (Some(host), Some(original)) = (
            normalized_host(host),
            match value.as_str() {
                Some("original") => Some(true),
                Some("translate") => Some(false),
                _ => None,
            },
        ) else {
            continue;
        };
        values.insert(host, original);
    }
    Some(values)
}

fn load_translation_sites() -> BTreeMap<String, bool> {
    std::fs::read_to_string(translation_sites_path())
        .ok()
        .and_then(|source| parse_translation_sites(&source))
        .or_else(|| parse_translation_sites(EMBEDDED_TRANSLATION_SITES))
        .unwrap_or_default()
}

/// 既定リストへ実行時の選択を重ねた一覧。
fn merged_translation_sites() -> BTreeMap<String, bool> {
    let mut sites = DEFAULT_ORIGINAL_HOSTS
        .iter()
        .map(|host| ((*host).to_string(), true))
        .collect::<BTreeMap<String, bool>>();
    if let Ok(stored) = TRANSLATION_SITES.lock() {
        for (host, original) in stored.iter() {
            sites.insert(host.clone(), *original);
        }
    }
    sites
}

fn translation_sites_script() -> String {
    let json = serde_json::to_string(&merged_translation_sites()).unwrap_or_else(|_| "{}".into());
    format!(";window.__cockpitTrSites = {json};\n")
}

fn remember_translation_site(host: &str, original: bool) {
    let Some(host) = normalized_host(host) else {
        return;
    };
    let stored = {
        let Ok(mut sites) = TRANSLATION_SITES.lock() else {
            return;
        };
        if sites.get(&host) == Some(&original) {
            return;
        }
        sites.insert(host, original);
        sites.clone()
    };
    let _ = save_translation_sites(&stored);
}

fn save_translation_sites(sites: &BTreeMap<String, bool>) -> Result<(), String> {
    let path = translation_sites_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("can't create {}: {error}", dir.display()))?;
    }
    write_translation_sites(&path, sites)
}

fn write_translation_sites(path: &Path, sites: &BTreeMap<String, bool>) -> Result<(), String> {
    let mut object = serde_json::Map::new();
    for (host, original) in sites {
        object.insert(
            host.clone(),
            serde_json::Value::String(if *original { "original" } else { "translate" }.to_string()),
        );
    }
    let mut source = serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "sites": object,
    }))
    .map_err(|error| format!("can't encode the translation sites: {error}"))?;
    source.push('\n');
    let temp = path.with_file_name(format!(
        ".browser-translation.json.tmp-{}",
        std::process::id()
    ));
    std::fs::write(&temp, source)
        .map_err(|error| format!("can't write the translation sites to {}: {error}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        format!("can't replace {}: {error}", path.display())
    })
}

/// 頁ごとの翻訳の選択の置き場(状態の置き場の中)。
fn translation_sites_path() -> PathBuf {
    armature_core::geo::state_dir().join("browser/translation-sites.json")
}

fn translation_site_message(body: &str) -> Option<SiteMessage> {
    let (verb, host) = body.strip_prefix("trsite:")?.split_once(':')?;
    let host = normalized_host(host)?;
    match verb {
        "ask" => Some(SiteMessage::Ask(host)),
        "src" => Some(SiteMessage::Set {
            host,
            original: true,
        }),
        "ja" => Some(SiteMessage::Set {
            host,
            original: false,
        }),
        _ => None,
    }
}

/// 選択を覚え、問い合わせには最新の判定を返す。
///
/// 頁へ埋めた一覧は、そのタブを作った時点のもの。すでに開いているタブが
/// あとから同じドメインへ移ったときのために、開くたびここで測り直す。
fn answer_translation_site(id: u64, message: &SiteMessage) {
    let host = match message {
        SiteMessage::Set { host, original } => {
            remember_translation_site(host, *original);
            return;
        }
        SiteMessage::Ask(host) => host,
    };
    let verdict = shows_original(&merged_translation_sites(), host);
    let script = format!("window.__cockpitTr && window.__cockpitTr.verdict({verdict});");
    MANAGER.with(|manager| {
        if let Ok(manager) = manager.try_borrow()
            && let Some(view) = manager.tabs.iter().find(|tab| tab.id == id).and_then(Tab::live)
        {
            let _ = view.evaluate_script(&script);
        }
    });
}

/// 頁に仕込む訳の JS。訳す先の言語と札の文言をいまの言語で差し込む。
/// 言語を切り替えたあとは、新しく開いた頁から効く。
fn translation_script() -> String {
    let labels = tr!(
        serde_json::json!({
            "noReply": "The translation didn't come back",
            "cantAsk": "Couldn't ask for a translation",
            "refining": "Claude is translating…",
            "translating": "Translating…",
            "refined": "Claude's translation (t for the original)",
            "translated": "English (t for the original / T for Claude's translation)",
            "failed": "Couldn't keep translating",
            "original": "Original (t for English)",
            "shown": "English (t for the original)",
        }),
        serde_json::json!({
            "noReply": "翻訳の返事が届かなかった",
            "cantAsk": "翻訳を頼めなかった",
            "refining": "精訳中…",
            "translating": "翻訳中…",
            "refined": "精訳 (t で原文)",
            "translated": "日本語 (t で原文 / T で精訳)",
            "failed": "翻訳を続けられなかった",
            "original": "原文 (t で日本語)",
            "shown": "日本語 (t で原文)",
        }),
    );
    TRANSLATION
        .replace("__ARMATURE_TARGET__", armature_core::lang::current().key())
        .replace("__ARMATURE_TR_LABELS__", &labels.to_string())
}

const TRANSLATION: &str = r#"
;(function () {
  if (window.top !== window) { return; }
  if (window.__cockpitTr) { return; }
  var BLOCK = 'p,li,h1,h2,h3,h4,h5,h6,dd,dt,td,th,blockquote,figcaption,summary,caption';
  var SKIP = {SCRIPT:1,STYLE:1,CODE:1,PRE:1,KBD:1,SAMP:1,NOSCRIPT:1,TEXTAREA:1,SVG:1};
  // 訳文は textContent の丸ごと置換で入れる。中に <style>/<script> を抱えた塊を掴むと、
  // その CSS/JS まで「本文」として訳されて頁の綴りが消え、訳した CSS が地の文として並ぶ
  // (2026-08-19 Amazon 商品頁で実測: `.centralizedApexPriceSavingsOverrides { フォントウェイト: 300! 重要; }`)。
  // 掴む前に外す。
  var FRAGILE = 'style,script,noscript,template,link,meta,svg,canvas,video,audio,iframe,object,embed,textarea,pre,code';
  var done = [], showing = 'src', running = false, badge = null, ask = 0, waiting = null;
  var autoRuns = 0, emptyRuns = 0, autoTimer = null, rerun = false, muting = false;
  var AUTO_RUN_LIMIT = 16, EMPTY_RUN_LIMIT = 8, AUTO_UNTIL = Date.now() + 120000;
  var nonce = Date.now().toString(36) + Math.random().toString(36).slice(2);
  var chosen = false;
  // 訳す先の言語と、札の文言。頁へ仕込むときに Rust が差し込む(`translation_script`)。
  var TARGET = '__ARMATURE_TARGET__';
  var L = __ARMATURE_TR_LABELS__;
  function host() { return String(location.hostname || '').toLowerCase(); }
  function original() {
    var map = window.__cockpitTrSites || {}, name = host(), best = -1, verdict = false;
    for (var entry in map) {
      if (!Object.prototype.hasOwnProperty.call(map, entry)) continue;
      if (name !== entry &&
          (name.length <= entry.length || name.slice(-(entry.length + 1)) !== '.' + entry)) continue;
      if (entry.length > best) { best = entry.length; verdict = !!map[entry]; }
    }
    return verdict;
  }
  var paused = original();
  function typing() {
    var a = document.activeElement;
    if (!a) { return false; }
    var tag = (a.tagName || '').toLowerCase();
    return a.isContentEditable || tag === 'input' || tag === 'textarea' || tag === 'select';
  }
  function say(value) {
    try { window.__cockpitSay(value); return true; }
    catch (_) { return false; }
  }
  function setBadge(value) {
    if (!badge) {
      badge = document.createElement('div');
      badge.style.cssText = 'position:fixed;right:10px;bottom:10px;z-index:2147483647;padding:4px 10px;border-radius:7px;pointer-events:none;font:11px/1.5 -apple-system,"Hiragino Sans",sans-serif;background:rgba(35,38,52,.94);color:#c6d0f5;box-shadow:0 2px 10px rgba(0,0,0,.4)';
      document.documentElement.appendChild(badge);
    }
    badge.textContent = value || ''; badge.style.display = value ? '' : 'none';
  }
  function fade(value) { setBadge(value); setTimeout(function () { setBadge(''); }, 2200); }
  function skipped(el) {
    for (var p = el; p && p !== document.documentElement; p = p.parentElement) {
      if (SKIP[p.tagName] || (p.getAttribute && p.getAttribute('translate') === 'no')) return true;
    }
    return !!(el.querySelector && el.querySelector(FRAGILE));
  }
  // 訳す値打ちがあるのは「英字を含み、まだ日本語になっていない」塊だけ。
  // 英字2文字だけを門にすると、日本語の頁に混じる英字の断片(CSS の名残・型番・商品名)
  // まで英→日に投げてしまい、返ってきた別物で地の文が置き換わる。
  function foreign(text) {
    if (TARGET !== 'ja') {
      // 英語へ訳すとき: ラテン文字でない字が2つ以上あれば外国語。ラテン文字だけの塊は、
      // 頁が英語以外を名乗っているときだけ(仏語・独語の頁)。
      if ((text.match(/[^\P{L}\p{Script=Latin}]/gu) || []).length >= 2) return true;
      var page = String(document.documentElement.lang || '').toLowerCase();
      return page !== '' && page.slice(0, 2) !== 'en' && /[A-Za-z]{2}/.test(text);
    }
    if (!/[A-Za-z]{2}/.test(text)) return false;
    if (/[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff]/.test(text)) return false;
    return true;
  }
  function collect() {
    var out = [], all = document.querySelectorAll(BLOCK);
    for (var i = 0; i < all.length && out.length < 400; i++) {
      var el = all[i], text = (el.textContent || '').trim();
      if (el.__trDone || el.querySelector(BLOCK) || skipped(el)) continue;
      if (text.length < 2 || text.length > 3000 || !foreign(text)) continue;
      out.push(el);
    }
    return out;
  }
  function candidate(node) {
    var el = node && (node.nodeType === 1 ? node : node.parentElement);
    if (!el) return false;
    var blocks = [];
    if (el.matches && el.matches(BLOCK)) blocks.push(el);
    if (el.querySelectorAll) blocks = blocks.concat(Array.prototype.slice.call(el.querySelectorAll(BLOCK)));
    for (var i = 0; i < blocks.length; i++) {
      var block = blocks[i], text = (block.textContent || '').trim();
      if (!block.__trDone && !skipped(block) && foreign(text)) return true;
    }
    return false;
  }
  function scheduleAuto(delay) {
    if (paused) return;
    if (Date.now() > AUTO_UNTIL || autoRuns >= AUTO_RUN_LIMIT) return;
    clearTimeout(autoTimer);
    autoTimer = setTimeout(function () { autoTimer = null; run('auto'); }, delay);
  }
  function send(mode, texts) {
    return new Promise(function (resolve) {
      ask += 1;
      var id = ask;
      var timer = setTimeout(function () {
        if (!waiting || waiting.id !== id) return;
        waiting = null;
        resolve({ok:false, texts:[], reason:L.noReply});
      }, 480000);
      waiting = {id:id, resolve:function (result) { clearTimeout(timer); resolve(result); }};
      if (!say('tr:' + nonce + ':' + id + ':' + JSON.stringify({mode:mode,texts:texts}))) {
        var current = waiting; waiting = null;
        current.resolve({ok:false, texts:[], reason:L.cantAsk});
      }
    });
  }
  async function run(mode) {
    if (paused) return;
    if (running) { if (mode === 'auto') rerun = true; return; }
    if (mode === 'auto') {
      if (Date.now() > AUTO_UNTIL || autoRuns >= AUTO_RUN_LIMIT) return;
      autoRuns += 1;
    }
    running = true;
    try {
      var els = collect();
      if (!els.length) {
        if (mode === 'auto' && emptyRuns < EMPTY_RUN_LIMIT) {
          emptyRuns += 1; scheduleAuto(Math.min(4000, emptyRuns * 500));
        }
        return;
      }
      emptyRuns = 0;
      setBadge(mode === 'claude' ? L.refining : L.translating);
      for (var i = 0; i < els.length; i += 20) {
        var part = els.slice(i, i + 20);
        var texts = part.map(function (el) { return el.textContent.trim().replace(/\s+/g, ' '); });
        var result = await send(mode, texts);
        if (paused) return;
        if (!result || !result.ok) { fade(result && result.reason ? result.reason : ''); return; }
        for (var j = 0; j < part.length; j++) {
          if (!result.texts[j]) continue;
          var el = part[j];
          if (el.__trHTML === undefined) el.__trHTML = el.innerHTML;
          el.__trJa = result.texts[j]; el.__trDone = true; el.textContent = result.texts[j]; done.push(el);
        }
        showing = 'ja';
        if (els.length > 20) setBadge(L.translating + ' ' + Math.min(i + 20, els.length) + '/' + els.length);
      }
      fade(mode === 'claude' ? L.refined : L.translated);
    } catch (_) {
      fade(L.failed);
    } finally {
      running = false; waiting = null;
      if (rerun) { rerun = false; scheduleAuto(400); }
    }
  }
  function paint(next) {
    muting = true;
    done.forEach(function (el) {
      if (next === 'ja') el.textContent = el.__trJa; else el.innerHTML = el.__trHTML;
    });
    queueMicrotask(function () { muting = false; });
    showing = next;
  }
  function resume() {
    AUTO_UNTIL = Date.now() + 120000; autoRuns = 0; emptyRuns = 0;
  }
  function setPaused(next, remember) {
    next = !!next;
    if (remember) { chosen = true; say('trsite:' + (next ? 'src' : 'ja') + ':' + host()); }
    if (next === paused) return;
    paused = next;
    if (paused) {
      clearTimeout(autoTimer); autoTimer = null; rerun = false;
      if (done.length) paint('src');
      fade(L.original);
    } else {
      resume();
      if (done.length) { paint('ja'); fade(L.shown); }
      observe(); scheduleAuto(0);
    }
  }
  window.__cockpitTr = {
    take:function (page, id, ok, texts, reason) {
      if (page !== nonce || !waiting || waiting.id !== id) return;
      var current = waiting; waiting = null; current.resolve({ok:ok,texts:texts,reason:reason});
    },
    verdict:function (value) { if (!chosen) setPaused(value, false); },
    toggle:function () { setPaused(!paused, true); },
    redo:function () {
      if (running) return;
      if (paused) { paused = false; resume(); observe(); }
      muting = true;
      done.forEach(function (el) { el.innerHTML = el.__trHTML; el.__trDone = false; });
      queueMicrotask(function () { muting = false; });
      done = []; showing = 'src'; run('claude');
    }
  };
  window.addEventListener('keydown', function (event) {
    if (event.metaKey || event.ctrlKey || event.altKey || typing()) return;
    if (event.key === 't') {
      window.__cockpitTr.toggle(); event.preventDefault();
    } else if (event.key === 'T') {
      window.__cockpitTr.redo(); event.preventDefault();
    }
  }, true);
  function observe() {
    var root = document.documentElement;
    if (!root || window.__cockpitTrObserver) return;
    window.__cockpitTrObserver = new MutationObserver(function (records) {
      if (muting) return;
      if (!records.some(function (record) {
        if (record.type === 'characterData') return candidate(record.target);
        return Array.prototype.some.call(record.addedNodes || [], candidate);
      })) return;
      if (running) rerun = true; else scheduleAuto(450);
    });
    window.__cockpitTrObserver.observe(root, {childList:true,subtree:true,characterData:true});
  }
  function start() {
    say('trsite:ask:' + host());
    if (paused) return;
    observe(); scheduleAuto(350);
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start);
  else start();
})();
"#;

struct Parent<'a>(&'a dyn iced::window::Window);

impl HasWindowHandle for Parent<'_> {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

/// タブを開いて前面に出す。同じ行き先のタブが既に在れば、そちらを選ぶ。
pub fn open(
    window: &dyn iced::window::Window,
    source: Source,
    bounds: Bounds,
) -> Result<Snapshot, String> {
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        // パネルの幅はここでも拾う。resize より先に最初のタブが生えると、幅を
        // 知らないまま天井の倍率で開いてしまう。
        manager.width = bounds.width;
        remember_parent(&mut manager, window, bounds);
        let identity = source.identity();
        if let Some(index) = manager.tabs.iter().position(|tab| tab.url == identity) {
            // md は開き直しのたびに組み直す。元の md を書き替えて同じ口で
            // 開き直したとき、古い版のまま残らないように。
            if let (Source::HtmlFile { html, .. }, Some(view)) = (&source, manager.tabs[index].live())
            {
                view.load_html(html)
                    .map_err(|error| format!("can't reload the page: {error}"))?;
            }
            select_inner(&mut manager, index, true)?;
            return Ok(snapshot(&manager));
        }

        let id = manager.next_id;
        manager.next_id += 1;
        let new_tab_nonce = match &source {
            Source::NewTab { serial, .. } => Some(*serial),
            _ => None,
        };
        let webview = build_view(&manager, &Parent(window), id, &source, bounds, false)?;
        manager.tabs.push(Tab {
            id,
            title: source.title(),
            url: identity,
            new_tab_nonce,
            surface: Surface::Live(webview),
        });
        let last = manager.tabs.len() - 1;
        select_inner(&mut manager, last, true)?;
        Ok(snapshot(&manager))
    })
}

/// 窓の生の手掛かり。控えのタブを起こす瞬間には `&dyn Window` が手元に
/// 無いので、最初に窓を渡されたときに写しておく。
struct RememberedParent(RawWindowHandle);

impl HasWindowHandle for RememberedParent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: 手掛かりはこのコックピットの窓のもので、窓はプロセスの
        // 終わりまで消えない。借用がぶら下がる相手が居なくなることはない。
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

/// 控えのタブを起こす手掛かり(親窓・パネルの位置)を覚える。窓を渡される口は
/// 全部ここを通す——どれが最初に呼ばれるかは起動の順で変わる。
fn remember_parent(manager: &mut Manager, window: &dyn iced::window::Window, bounds: Bounds) {
    if let Ok(handle) = window.window_handle() {
        manager.parent = Some(handle.as_raw());
    }
    manager.bounds = Some(bounds);
}

/// 頁に載せる仕掛け一式を1本に組む。
///
/// **先頭は必ず [`SAY`]。** 後ろの仕掛けはどれも `__cockpitSay` で物を言うので、
/// 口が先に立っていないと、頁が始まった直後の伝言(題・行き先)だけが落ちる。
fn page_script(source: &Source, hush: bool) -> String {
    let ad_block = if matches!(source, Source::Url(_)) {
        AD_BLOCK
    } else {
        ""
    };
    let hush = if hush { HUSH } else { "" };
    // ドラフト(file://)の中のリンクだけ新しいタブへ逃がす。
    let links = local_link_script(source);
    if source.translates() {
        let sites = translation_sites_script();
        format!("{SAY}{PAGE_SURFACE}{SWIPE}{ad_block}{KEYS}{HINTS}{hush}{sites}{}", translation_script())
    } else {
        format!("{SAY}{PAGE_SURFACE}{SWIPE}{ad_block}{KEYS}{HINTS}{hush}{links}")
    }
}

/// WebView を1枚組む。開く口(`open_with`)と起こす口(`wake`)の共通部分で、
/// タブの台帳には触らない。`hush` は起こし直した頁に載せる自動再生の見張り。
fn build_view<W: HasWindowHandle>(
    manager: &Manager,
    parent: &W,
    id: u64,
    source: &Source,
    bounds: Bounds,
    hush: bool,
) -> Result<WebView, String> {
    use wry::WebViewBuilderExtDarwin;

    let script = page_script(source, hush);
    let ipc_id = id;
    // 伝言の口(`window.ipc`)は wry のものを使わず、[`ipc::install`] が据える。
    // wry の口は送り主のフレームの URL しか渡さないので、外の iframe が立てた
    // `about:blank` の枠から言われると器の頁の伝言と見分けられない。
    let builder = WebViewBuilder::new()
        .with_initialization_script(&script)
        // **target=_blank は同じタブで開く。** 新しいタブを生やすと、その
        // タブは `with_url` で生まれるので履歴が空になり、‹ が何も返さない
        // ——利用者からは「戻ることすらできない」に見える(2026-08-23実測:
        // タブ27枚・全件 canGoBack=false)。同じタブに載せ替えれば履歴が
        // 溜まり、普通のブラウザと同じ戻り方になる。
        //
        // 外から渡すもの(⌘クリック・栞)は今までどおり新しいタブ。
        // 別の口を通るので、ここの変更は掛からない。
        .with_new_window_req_handler(|url, _| {
            // 頁が開かせてよいのは web の頁だけ。file:// をタブの行き先に据えさせない
            // (「外で開く」で手元のアプリまで起きる)。
            if is_web_url(&url) && !is_ad_url(&url) {
                queue(Some(Request::NavigateActive(url)));
            }
            wry::NewWindowResponse::Deny
        })
        // 広告の配信元へは行かせない。
        .with_navigation_handler(|url| !is_ad_url(&url))
        // カメラ・マイクは頁ごとに WebKit の確認を出す。渡さないと wry は黙って許可する。
        .with_permission_handler(|_| wry::PermissionResponse::Default)
        .with_user_agent(USER_AGENT)
        .with_data_store_identifier(DATA_STORE)
        // 作り付けの身振りは切る。自前の [`SWIPE`] が主で、両方生かすと
        // 一度の払いで二枚戻る(2026-08-23)。
        .with_back_forward_navigation_gestures(false)
        // **音は利用者が押したときだけ。**窓を起こし直すとタブが復元され、
        // 動画の頁を開いたままだった日は勝手に鳴り出していた。再生ボタンを押せば普通に鳴る。
        .with_autoplay(false)
        // **地はいつも窓と同じ色で生まれる。**明るい紙を最初から持たせると、
        // 頁が描かれるまでの一瞬だけ白く光る。2026-08-22 に自分で描く頁だけ
        // 直したが、外のサイトを開くタブには紙が残っていた。
        //
        // 紙は [`ground_for`] が読み込みの終わりに敷く——背景を持たない頁の
        // 黒文字を読めなくしない役目はそのまま残る。
        .with_background_color(page_ground())
        .with_on_page_load_handler(move |event, url| {
            if let Some(colour) = ground_for(&event, &url) {
                ground(ipc_id, colour);
            }
            // 読み終えた頁の実の URL をアドレス欄へ。頁が自分の知らせを止めても追える。
            if matches!(event, wry::PageLoadEvent::Finished) && !url.is_empty() {
                queue(Some(Request::Location { id: ipc_id, url }));
            }
        })
        .with_bounds(rect(bounds));
    if let Source::Url(url) = source
        && !loadable(url)
    {
        return Err(tr!(format!("Not an address: {url}"), format!("行き先として読めない: {url}")).to_string());
    }
    let builder = match source {
        Source::Url(url) => builder.with_url(url),
        Source::HtmlFile { html, .. } | Source::NewTab { html, .. } => builder.with_html(html),
    };
    let webview = builder
        .build_as_child(parent)
        .map_err(|error| tr!(format!("Couldn't create the browser: {error}"), format!("中央ブラウザを作れない: {error}")))?;
    // 読み込みは窓の糸へ戻ってから進むので、頁の script が走る前に口が据わる。
    ipc::install(&webview, id);
    raise(&webview);
    // 開いた頁の字を窓の倍率へ合わせる。倍率は font 側の正本から引く
    // ——タブは窓より後から生えるので、押された瞬間の値を待てない。
    // setPageZoom は失敗しない口なので、拡大の不首尾でタブは捨てない。
    let _ = webview.zoom(pending_zoom(manager));
    Ok(webview)
}

/// 控えのタブを起こす。起きていれば何もしない。
///
/// 復元で並べた札は WebView を持たない。初めて選ばれた瞬間にここで組む
/// ——手を触れていないタブのために WebKit のプロセスを抱えない。
fn wake(manager: &mut Manager, index: usize) -> Result<(), String> {
    let Some(tab) = manager.tabs.get(index) else {
        return Ok(());
    };
    if !tab.asleep() {
        return Ok(());
    }
    let parent = manager
        .parent
        .ok_or_else(|| "no window to restore the tab into".to_string())?;
    let bounds = manager
        .bounds
        .ok_or_else(|| "no place to restore the tab into".to_string())?;
    let id = tab.id;
    let source = source_for_saved_url(&tab.url);
    // 起こし直した頁なので、手を触れるまで鳴らさない見張りを載せる。
    let view = build_view(manager, &RememberedParent(parent), id, &source, bounds, true)?;
    view.set_visible(false)
        .map_err(|error| format!("can't hide the restored tab: {error}"))?;
    manager.tabs[index].surface = Surface::Live(view);
    Ok(())
}

pub fn revive(
    window: &dyn iced::window::Window,
    tabs: Vec<TabInfo>,
    active: usize,
    bounds: Bounds,
) -> Result<Snapshot, String> {
    // **復元では WebView を1枚も作らない。**保存済みの札を並べるだけにして、
    // 初めて選ばれたタブから [`wake`] で起こす。全部を一斉に起こしていた頃は
    // WebKit のプロセスが 70 本・1.5GB 生まれ、窓が立った直後の機体が地面を
    // 擦っていた。
    //
    // 自動再生の見張り([`HUSH`])は起こす側が載せる——`with_autoplay(false)`
    // は macOS では既定を上書きしないので効かない(2026-08-22 実測)。
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        manager.width = bounds.width;
        remember_parent(&mut manager, window, bounds);
        let mut active_index = None;
        for (index, saved) in tabs.into_iter().enumerate() {
            if !is_persistable_url(&saved.url) {
                continue;
            }
            if manager.tabs.iter().any(|tab| tab.url == saved.url) {
                continue;
            }
            let id = manager.next_id;
            manager.next_id += 1;
            manager.tabs.push(Tab {
                id,
                title: saved.title,
                url: saved.url,
                new_tab_nonce: None,
                surface: Surface::Asleep,
            });
            if index == active {
                active_index = Some(manager.tabs.len() - 1);
            }
        }
        if let Some(index) = active_index {
            manager.active = index;
        }
        // 面は伏せたまま返す。端末を見ている窓に頁を被せない。
        manager.visible = false;
        Ok(snapshot(&manager))
    })
}

/// 手を触れるまで自動再生を止める見張り。押した・打った時点で解ける。
const HUSH: &str = r#"
;(function () {
  if (window.__cockpitHush) { return; }
  window.__cockpitHush = true;
  var free = false;
  ['pointerdown', 'keydown'].forEach(function (name) {
    addEventListener(name, function () { free = true; }, true);
  });
  function hush(event) {
    if (free) { return; }
    var el = event.target;
    if (el && typeof el.pause === 'function') { el.pause(); }
  }
  document.addEventListener('play', hush, true);
  // 見張るのは起こし直した直後だけ。しばらく放っておかれたら手を引く。
  setTimeout(function () { document.removeEventListener('play', hush, true); }, 60000);
})();
"#;

#[must_use]
pub fn load_saved() -> Snapshot {
    let Ok(content) = std::fs::read_to_string(pages_path()) else {
        return Snapshot::default();
    };
    parse_saved(&content)
}

fn parse_saved(content: &str) -> Snapshot {
    let mut active = 0;
    let mut tabs = Vec::new();
    for line in content.lines() {
        let mut fields = line.split('\t');
        let Some(title) = fields.next() else {
            continue;
        };
        let Some(url) = fields.next() else {
            continue;
        };
        if !is_persistable_url(url) {
            continue;
        }
        let index = tabs.len();
        if fields.next().is_some_and(|value| value.trim() == "1") {
            active = index;
        }
        tabs.push(TabInfo {
            id: u64::try_from(index).unwrap_or(0),
            title: title.trim().to_string(),
            url: url.trim().to_string(),
            asleep: true,
        });
    }
    if active >= tabs.len() {
        active = 0;
    }
    Snapshot {
        tabs,
        active,
        visible: false,
    }
}

pub fn save(snapshot: &Snapshot) {
    let mut body = String::new();
    for (index, tab) in snapshot.tabs.iter().enumerate() {
        if !is_persistable_url(&tab.url) {
            continue;
        }
        body.push_str(&format!(
            "{}\t{}\t{}\n",
            tab.title.replace(['\t', '\n'], " "),
            tab.url.replace(['\t', '\n'], " "),
            u8::from(index == snapshot.active)
        ));
    }
    let path = pages_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, body);
}

/// 現在の頁を栞へ追加する。
pub fn bookmark_active() -> Result<(), String> {
    let (title, url) = MANAGER.with(|manager| {
        let manager = manager.borrow();
        let tab = manager
            .tabs
            .get(manager.active)
            .ok_or_else(|| tr!("No browser tab to bookmark", "栞に入れるブラウザタブが無い").to_string())?;
        if !is_persistable_url(&tab.url) || is_internal_page(&tab.url) {
            return Err(String::from(tr!("This page can't be bookmarked", "この内部頁は栞に入れられない")));
        }
        Ok((tab.title.clone(), tab.url.clone()))
    })?;
    let path = armature_core::geo::state_dir().join("browser/bookmarks.json");
    let _guard = BOOKMARK_LOCK
        .lock()
        .map_err(|_| "the bookmark lock is poisoned".to_string())?;
    let mut items = std::fs::read_to_string(&path)
        .ok()
        .and_then(|source| serde_json::from_str::<serde_json::Value>(&source).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    if items
        .iter()
        .any(|item| item.get("spec").and_then(serde_json::Value::as_str) == Some(&url))
    {
        return Ok(());
    }
    items.push(serde_json::json!({ "title": title, "spec": url }));
    write_bookmarks(&path, &items)
}

/// 新規タブの削除ボタンから、選ばれた栞だけを永続一覧から外す。
pub fn delete_bookmark(id: u64, nonce: u64, spec: &str) -> Result<(), String> {
    ensure_new_tab_request(id, nonce)?;
    let path = armature_core::geo::state_dir().join("browser/bookmarks.json");
    let _guard = BOOKMARK_LOCK
        .lock()
        .map_err(|_| "the bookmark lock is poisoned".to_string())?;
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(tr!(format!("Can't read the bookmarks: {error}"), format!("栞を読めない: {error}"))),
    };
    let mut items = serde_json::from_str::<Vec<serde_json::Value>>(&source)
        .map_err(|error| format!("can't parse the bookmarks: {error}"))?;
    if !remove_bookmark_value(&mut items, spec) {
        return Ok(());
    }
    write_bookmarks(&path, &items)?;
    let spec =
        serde_json::to_string(spec).map_err(|error| format!("can't encode the bookmark reply: {error}"))?;
    let script =
        format!("window.__cockpitBookmarkDeleted&&window.__cockpitBookmarkDeleted({spec});");
    MANAGER.with(|manager| {
        let manager = manager.borrow();
        let view = manager
            .tabs
            .iter()
            .find(|tab| tab.id == id)
            .and_then(Tab::live)
            .ok_or_else(|| "the tab that removed the bookmark is gone".to_string())?;
        view.evaluate_script(&script)
            .map_err(|error| format!("can't refresh the bookmarks: {error}"))
    })
}

pub fn open_bookmark(id: u64, nonce: u64, url: &str) -> Result<(), String> {
    ensure_new_tab_request(id, nonce)?;
    let Some(url) = bookmark_target(url) else {
        return Err(tr!("This bookmark can't be opened", "開けない栞です").into());
    };
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        let tab = manager
            .tabs
            .iter_mut()
            .find(|tab| tab.id == id)
            .ok_or_else(|| "no browser tab to open the bookmark in".to_string())?;
        let view = tab
            .live()
            .ok_or_else(|| "the browser tab for the bookmark is not loaded".to_string())?;
        load(view, &url).map_err(|error| tr!(format!("Can't open the bookmark: {error}"), format!("栞を開けない: {error}")))?;
        tab.url = url;
        tab.new_tab_nonce = None;
        Ok(())
    })
}

/// 開いている資料を、タブはそのままに最新の中身へ差し替える。
///
/// [`open`] は同じ url のタブが在れば**選ぶだけで中身に触らない**ので、
/// 発行し直した成果物はこちらから載せ替える。既に閉じられていたら何もしない
/// ——勝手に開き直すと、閉じたはずの資料が戻ってくる。
pub fn reload_source(source: &Source) -> Result<(), String> {
    let identity = source.identity();
    MANAGER.with(|manager| {
        let manager = manager.borrow();
        let Some(tab) = manager.tabs.iter().find(|tab| tab.url == identity) else {
            return Ok(());
        };
        // 控えは起きるときに最新の中身を読むので、ここで触る面が無い。
        let Some(view) = tab.live() else {
            return Ok(());
        };
        match source {
            Source::Url(url) => load(view, url),
            Source::HtmlFile { html, .. } | Source::NewTab { html, .. } => view.load_html(html),
        }
        .map_err(|error| format!("can't reload the page: {error}"))
    })
}

fn ensure_new_tab_request(id: u64, nonce: u64) -> Result<(), String> {
    MANAGER.with(|manager| {
        manager
            .borrow()
            .tabs
            .iter()
            .any(|tab| tab.id == id && tab.new_tab_nonce == Some(nonce))
            .then_some(())
            .ok_or_else(|| "bookmarks can only be changed from the new-tab page".to_string())
    })
}

fn write_bookmarks(path: &Path, items: &[serde_json::Value]) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err("no place to keep the bookmarks".into());
    };
    std::fs::create_dir_all(parent).map_err(|error| format!("can't create the bookmark folder: {error}"))?;
    let nonce = NEXT_BOOKMARK_WRITE.fetch_add(1, Ordering::Relaxed);
    let next = path.with_file_name(format!(".bookmarks.{}.{nonce}.next", std::process::id()));
    let body =
        serde_json::to_vec_pretty(items).map_err(|error| format!("can't encode the bookmarks: {error}"))?;
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&next)
            .map_err(|error| format!("can't create the bookmark temp file: {error}"))?;
        file.write_all(&body)
            .map_err(|error| format!("can't write the bookmarks: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("can't sync the bookmarks: {error}"))?;
        std::fs::rename(&next, path).map_err(|error| format!("can't replace the bookmarks: {error}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(next);
    }
    result
}

fn remove_bookmark_value(items: &mut Vec<serde_json::Value>, spec: &str) -> bool {
    let before = items.len();
    items.retain(|item| {
        item.get("spec")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|value| value != spec)
    });
    items.len() != before
}

pub fn show(bounds: Bounds) -> Result<Snapshot, String> {
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        manager.bounds = Some(bounds);
        if manager.tabs.is_empty() {
            return Ok(snapshot(&manager));
        }
        for view in manager.tabs.iter().filter_map(Tab::live) {
            view.set_bounds(rect(bounds))
                .map_err(|error| format!("can't place the browser: {error}"))?;
        }
        let active = manager.active;
        select_inner(&mut manager, active, true)?;
        Ok(snapshot(&manager))
    })
}

pub fn hide() -> Result<Snapshot, String> {
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        for view in manager.tabs.iter().filter_map(Tab::live) {
            view.set_visible(false)
                .map_err(|error| format!("can't hide the browser: {error}"))?;
        }
        if let Some(view) = manager.tabs.get(manager.active).and_then(Tab::live) {
            let _ = view.focus_parent();
        }
        manager.visible = false;
        Ok(snapshot(&manager))
    })
}

pub fn select(index: usize) -> Result<Snapshot, String> {
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        select_inner(&mut manager, index, true)?;
        Ok(snapshot(&manager))
    })
}

pub fn cycle(step: i32) -> Result<Snapshot, String> {
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        if manager.tabs.is_empty() {
            return Ok(snapshot(&manager));
        }
        let count = i32::try_from(manager.tabs.len()).unwrap_or(1);
        let active = i32::try_from(manager.active).unwrap_or(0);
        let next = usize::try_from((active + step).rem_euclid(count)).unwrap_or(0);
        select_inner(&mut manager, next, true)?;
        Ok(snapshot(&manager))
    })
}

pub fn close_active() -> Result<Snapshot, String> {
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        if manager.tabs.is_empty() {
            return Ok(snapshot(&manager));
        }
        let active = manager.active;
        // 次のWebViewを出すまで現在の子Viewを生かしておく。先にdropすると、
        // AppKitが次を表示するまでの一瞬だけ背後のterminalが露出する。
        let closing = manager.tabs.remove(active);
        if manager.tabs.is_empty() {
            manager.active = 0;
            manager.visible = false;
        } else {
            let next = active.min(manager.tabs.len() - 1);
            select_inner(&mut manager, next, true)?;
        }
        drop(closing);
        Ok(snapshot(&manager))
    })
}

pub fn resize(bounds: Bounds) -> Result<(), String> {
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        manager.width = bounds.width;
        manager.bounds = Some(bounds);
        for view in manager.tabs.iter().filter_map(Tab::live) {
            view.set_bounds(rect(bounds))
                .map_err(|error| format!("can't place the browser: {error}"))?;
        }
        // パネルの幅が変われば収まる拡大率も変わる。置き直しと同時に掛け直す。
        apply_zoom(&mut manager)
    })
}

/// いま覚えているパネルの矩形。まだ一度も置いていなければ `None`。
#[must_use]
pub fn placed_bounds() -> Option<Bounds> {
    MANAGER.with(|manager| manager.try_borrow().ok().and_then(|manager| manager.bounds))
}

/// 1点でも食い違えばずれと見る。丸めの端数(1点未満)は数えない。
fn drifted(have: Bounds, want: Bounds) -> bool {
    const SLACK: f64 = 1.0;
    (have.x - want.x).abs() >= SLACK
        || (have.y - want.y).abs() >= SLACK
        || (have.width - want.width).abs() >= SLACK
        || (have.height - want.height).abs() >= SLACK
}

/// パネルの矩形が窓とずれていたら置き直す。**時計の刻みから呼ぶ自己修復。**
///
/// 控えの矩形は取り落としで古びる——復元の便([`revive`])は窓が立った時点の
/// 寸法を覚えるが、その直後の全画面復元と競る。負けた回は窓が生まれた頃の
/// 1440×900 の枠のまま頁が起き、**パネルの左上に小さく寄って出る**。置き直しの便を取り落としても、
/// 次の刻みで揃う。
pub fn reconcile(want: Bounds) -> Result<(), String> {
    let Some(have) = placed_bounds() else {
        return Ok(());
    };
    if !drifted(have, want) {
        return Ok(());
    }
    resize(want)
}

/// 頁が机いっぱいに広がるための、望ましい版面の幅(CSSピクセル)。
///
/// **ここを下回ると多くのサイトが「狭い画面」の並びへ折り返す。** 中央のパネルは
/// 窓の半分しかないので、窓の文字倍率をそのまま頁へ掛けると版面が 900px 台まで
/// 落ちて、机向けに組まれた頁が横にはみ出す。1280 は机向けの並びが崩れない下限として広く使われる値。
const FIT_WIDTH: f64 = 1280.0;

/// 頁を縮めてよい下限。これ以上絞ると字が読めない。
const MIN_FIT_ZOOM: f64 = 0.5;

/// 頁の拡大率を窓の文字倍率(⌘+/⌘-)へ合わせる。開いている全タブに掛け、
/// **これから開くタブのために覚える**——同じ窓の中で頁だけ字の大きさが
/// 違うのを避ける。
pub fn set_zoom(scale: f32) -> Result<(), String> {
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        manager.scale = Some(scale);
        apply_zoom(&mut manager)
    })
}

/// いまのパネルの幅と文字倍率から実効の拡大率を出し、全タブへ掛けて覚える。
fn apply_zoom(manager: &mut Manager) -> Result<(), String> {
    let scale = manager.scale.unwrap_or_else(crate::font::load_scale);
    let zoom = fit_zoom(scale, manager.width);
    manager.zoom = Some(zoom);
    for view in manager.tabs.iter().filter_map(Tab::live) {
        view.zoom(zoom)
            .map_err(|error| format!("can't zoom the browser: {error}"))?;
    }
    Ok(())
}

/// 窓の文字倍率を、パネルに収まる範囲へ押さえた頁の拡大率にする。
///
/// 利用者が選んだ倍率は**天井**として扱う——⌘+ で頁がパネルからはみ出すのは、
/// 字が大きくなる利より読めなくなる害のほうが大きい。パネルが広ければ天井まで
/// 使い、狭ければ [`FIT_WIDTH`] の版面が入るところまで絞る。
fn fit_zoom(scale: f32, width: f64) -> f64 {
    let ceiling = page_zoom(scale);
    if !(width.is_finite() && width > 1.0) {
        // パネルの幅をまだ知らない(窓より先に生えたタブ)。天井をそのまま使う。
        return ceiling;
    }
    // 天井が下限を割る値で来ても落とさない。`clamp` は min>max で panic する。
    let floor = MIN_FIT_ZOOM.min(ceiling);
    (width / FIT_WIDTH).clamp(floor, ceiling)
}

/// 窓の文字倍率を頁の拡大率へ。**同じ数字で揃える**——別々の刻みにすると
/// ⌘+ を押すたびにどちらかが置いていかれる。頭打ちは WebKit が壊れない範囲。
fn page_zoom(scale: f32) -> f64 {
    f64::from(if scale.is_finite() {
        scale.clamp(0.25, 5.0)
    } else {
        1.0
    })
}

/// これから生えるタブに掛ける拡大率。窓より後から生えるタブは押された瞬間の
/// 倍率を待てないので、覚えてある倍率(無ければ font 側の正本)と、いまの
/// パネルの幅から引き直す。**`MANAGER` を借り直さない**——呼び手は既に借りている。
fn pending_zoom(manager: &Manager) -> f64 {
    fit_zoom(
        manager.scale.unwrap_or_else(crate::font::load_scale),
        manager.width,
    )
}

/// アドレス欄に打った字の行き先へ、いま見ているタブを差し替える。
///
/// 打った字の解釈は新規タブの欄と同じ [`typed_target`]——URL か探し物かの
/// 判断を2か所に置くと、片方だけ賢い欄ができてしまう。
pub fn navigate_active(typed: &str) -> Result<Snapshot, String> {
    let Some(url) = typed_target(typed) else {
        return Err(tr!("Nothing to open", "行き先が空です").into());
    };
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        let active = manager.active;
        {
            let tab = manager
                .tabs
                .get_mut(active)
                .ok_or_else(|| tr!("No tab is open", "開いているタブが無い").to_string())?;
            // 控えなら行き先だけ差し替える。起きるときにその頁を読む。
            if let Some(view) = tab.live() {
                load(view, &url).map_err(|error| tr!(format!("Can't open: {error}"), format!("開けない: {error}")))?;
            }
            tab.url = url;
            tab.new_tab_nonce = None;
        }
        Ok(snapshot(&manager))
    })
}

/// 履歴を1つ戻る(負)/進む(正)。頁の外から history を叩くだけで、
/// 行き先の判断は頁に任せる。
/// 「戻る/進む」。**頁の中の履歴だけを辿る。タブは渡らない。**
///
/// 一度タブ渡りを混ぜたが、で撤回。戻り先が無いのは履歴が空だからで、そちらは「頁の中で開く」
/// (`NewWindowResponse::Allow` を使わず同じタブへ載せ替える)側で解いた。
pub fn history_step(delta: i32) -> Result<Snapshot, String> {
    MANAGER.with(|manager| {
        let manager = manager.borrow();
        let Some(view) = manager.tabs.get(manager.active).and_then(Tab::live) else {
            return Ok(snapshot(&manager));
        };
        if delta < 0 {
            view.go_back()
        } else {
            view.go_forward()
        }
        .map_err(|error| format!("can't go through the history: {error}"))?;
        Ok(snapshot(&manager))
    })
}

/// いま見ている頁を読み直す。
///
/// md は組み立てた HTML を載せているので `location.reload()` では元の md を
/// 読み直さない。md のタブだけは組み直して載せ替える。
pub fn reload_active() -> Result<(), String> {
    MANAGER.with(|manager| {
        let manager = manager.borrow();
        let Some(tab) = manager.tabs.get(manager.active) else {
            return Ok(());
        };
        let Some(view) = tab.live() else {
            return Ok(());
        };
        match source_for_url(&tab.url) {
            Source::HtmlFile { html, .. } => view.load_html(&html),
            _ => view.evaluate_script("location.reload()"),
        }
        .map_err(|error| tr!(format!("Can't reload the page: {error}"), format!("頁を読み直せない: {error}")))
    })
}

/// いま見ている頁を macOS の既定の開き手(ブラウザ)で開く。
/// 。
/// md のタブは組み立てた HTML を
/// 一時置き場へ書いてから渡す——素の md を渡すと外のブラウザでも字が化ける。
/// 一時置き場は元の md の外(md の隣に .html を撒かない)。
pub fn open_active_externally() -> Result<(), String> {
    let url = MANAGER.with(|manager| {
        let manager = manager.borrow();
        manager
            .tabs
            .get(manager.active)
            .map(|tab| tab.url.clone())
            .ok_or_else(|| tr!("No tab to open elsewhere", "外で開くタブが無い").to_string())
    })?;
    let target = external_target(&url, &std::env::temp_dir().join("cockpit-external"))?;
    // `--` の後ろに置くので、`-` で始まる綴りも `open` の引数としては読まれない。
    let mut command = std::process::Command::new("/usr/bin/open");
    command.arg("--").arg(&target);
    armature_core::proc::spawn_reaped_quiet(command)
        .map(drop)
        .map_err(|error| tr!(format!("Can't open the default browser: {error}"), format!("外のブラウザを起こせない: {error}")))
}

/// 外へ渡す綴り。http(s) と md 以外の手元の頁はそのまま、md は `dir` へ
/// 書いた HTML の道筋。器の内部頁(新規タブ)は外へ出す意味が無いので断る。
fn external_target(url: &str, dir: &Path) -> Result<String, String> {
    if !is_persistable_url(url) || is_internal_page(url) {
        return Err(tr!("This page can't be opened in another browser", "この頁は外のブラウザで開けない").to_string());
    }
    let Source::HtmlFile { html, title, .. } = source_for_url(url) else {
        return Ok(url.to_string());
    };
    std::fs::create_dir_all(dir).map_err(|error| format!("can't create the temp folder: {error}"))?;
    let path = dir.join(format!("{title}.html"));
    std::fs::write(&path, html).map_err(|error| format!("can't write the temp HTML: {error}"))?;
    Ok(path.to_string_lossy().into_owned())
}

pub fn focus() -> Result<(), String> {
    MANAGER.with(|manager| {
        let manager = manager.borrow();
        if let Some(view) = manager.tabs.get(manager.active).and_then(Tab::live) {
            view.focus()
                .map_err(|error| format!("can't focus the browser: {error}"))?;
        }
        Ok(())
    })
}

pub fn blur() -> Result<(), String> {
    MANAGER.with(|manager| {
        let manager = manager.borrow();
        if let Some(view) = manager.tabs.get(manager.active).and_then(Tab::live) {
            view.focus_parent()
                .map_err(|error| format!("can't take the focus back from the browser: {error}"))?;
        }
        Ok(())
    })
}

pub fn update(id: u64, title: Option<String>, url: Option<String>) -> Snapshot {
    MANAGER.with(|manager| {
        let mut manager = manager.borrow_mut();
        if let Some(tab) = manager.tabs.iter_mut().find(|tab| tab.id == id) {
            if let Some(title) = title.filter(|title| !title.trim().is_empty()) {
                tab.title = title;
            }
            if let Some(url) = url.filter(|url| !url.trim().is_empty()) {
                tab.url = accepted_location(&tab.url, &url);
                if is_persistable_url(&tab.url) {
                    tab.new_tab_nonce = None;
                    // URLが確まる唯一の点。ここで控えておくと、次に新規タブを
                    // 開いた欄で候補として出る。同じ行き先の連投は控え側が
                    // 「先頭へ動かす」で吸収する。
                    history::remember(&tab.url);
                }
            }
        }
        snapshot(&manager)
    })
}

fn accepted_location(current: &str, reported: &str) -> String {
    if current.starts_with("file:///") && !is_persistable_url(reported) {
        current.to_string()
    } else {
        reported.to_string()
    }
}

pub fn deliver(reply: &TranslationReply) -> Result<(), String> {
    MANAGER.with(|manager| {
        let manager = manager.borrow();
        let Some(view) = manager
            .tabs
            .iter()
            .find(|tab| tab.id == reply.id)
            .and_then(Tab::live)
        else {
            return Ok(());
        };
        view.evaluate_script(&reply.script)
            .map_err(|error| format!("can't hand the translation to the page: {error}"))
    })
}

pub fn translate(
    id: u64,
    nonce: String,
    ask: u32,
    claude: bool,
    texts: Vec<String>,
) -> TranslationReply {
    let (ok, got, reason) = if claude {
        match armature_core::page_trans::translate_with_claude(&texts) {
            Ok(got) => (true, got, String::new()),
            Err(reason) => (false, Vec::new(), reason),
        }
    } else {
        // 自動翻訳は端末内のApple Translationだけを使う。失敗時にGTXへ
        // 送る予備経路は、本文をGoogleへ渡すため利用者の方針に反する。
        let translated = crate::apple_translation::translate(&texts);
        automatic_translation_reply(&texts, translated)
    };
    let texts = serde_json::to_string(&got).unwrap_or_else(|_| "[]".into());
    let reason = serde_json::to_string(&reason).unwrap_or_else(|_| "\"\"".into());
    let nonce = serde_json::to_string(&nonce).unwrap_or_else(|_| "\"\"".into());
    TranslationReply {
        id,
        script: format!(
            "window.__cockpitTr && window.__cockpitTr.take({nonce}, {ask}, {ok}, {texts}, {reason});"
        ),
    }
}

fn automatic_translation_reply(
    original: &[String],
    result: Result<armature_core::page_trans::Translated, String>,
) -> (bool, Vec<String>, String) {
    match result {
        // 言語判定は頁全体ではなく、JSが送った最大20要素の束ごとに返る。
        // 日本語の束を「失敗」にするとJSが後続の束まで打ち切るため、原文を
        // 成功として返してこの束だけ変更せず先へ進める。
        Ok(got) if armature_core::page_trans::is_target_language(&got.lang) => {
            (true, original.to_vec(), String::new())
        }
        Ok(got) => (true, got.texts, String::new()),
        Err(reason) => (false, Vec::new(), reason),
    }
}

/// 捌く用事が溜まっているか。**読まずに見るだけ**——見張りの便を出すかどうかの判定に使う。
#[must_use]
pub fn has_requests() -> bool {
    REQUESTS.lock().map(|r| !r.is_empty()).unwrap_or(false)
}

pub fn take_requests() -> Vec<Request> {
    REQUESTS
        .lock()
        .map(|mut requests| std::mem::take(&mut *requests))
        .unwrap_or_default()
}

/// `%E7%86%B1` を字へ戻す。file:// の日本語の道筋は % で届く。
fn percent_decoded(url: &str) -> String {
    let bytes = url.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = (bytes[at] == b'%')
            .then(|| url.get(at + 1..at + 3))
            .flatten()
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        if let Some(byte) = hex {
            out.push(byte);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn queue(request: Option<Request>) {
    if let Some(request) = request
        && let Ok(mut requests) = REQUESTS.lock()
    {
        requests.push(request);
    }
}

fn parse(id: u64, body: &str) -> Option<Request> {
    // 新規タブの欄に打った字。URL か探し物かの判断はRust側で持つ
    // (頁のJSに置くと試験が書けない)。開く口は栞と同じ`OpenBookmark`
    // ——どちらも「いま見ている新規タブを行き先へ差し替える」1つの動き。
    if let Some(rest) = body.strip_prefix("navigate:") {
        let (nonce, typed) = rest.split_once(':')?;
        return Some(Request::OpenBookmark {
            id,
            nonce: nonce.parse::<u64>().ok()?,
            url: typed_target(typed)?,
        });
    }
    if let Some(rest) = body.strip_prefix("bookmark:open:") {
        let (nonce, url) = rest.split_once(':')?;
        let nonce = nonce.parse::<u64>().ok()?;
        let url = url.trim();
        if url.is_empty() {
            return None;
        }
        return Some(Request::OpenBookmark {
            id,
            nonce,
            url: url.to_string(),
        });
    }
    if let Some(rest) = body.strip_prefix("bookmark:delete:") {
        let (nonce, spec) = rest.split_once(':')?;
        let nonce = nonce.parse::<u64>().ok()?;
        if spec.is_empty() {
            return None;
        }
        return Some(Request::DeleteBookmark {
            id,
            nonce,
            spec: spec.to_string(),
        });
    }
    if let Some(title) = body.strip_prefix("title:") {
        return Some(Request::Title {
            id,
            title: title.to_string(),
        });
    }
    if let Some(url) = body.strip_prefix("location:") {
        return Some(Request::Location {
            id,
            url: url.to_string(),
        });
    }
    if let Some(url) = body.strip_prefix("tab:open:") {
        let url = url.trim();
        if url.is_empty() {
            return None;
        }
        return Some(Request::NewTab(url.to_string()));
    }
    if let Some(index) = body.strip_prefix("tab:index:") {
        return index
            .parse::<usize>()
            .ok()
            .and_then(|index| index.checked_sub(1))
            .map(Request::SelectTab);
    }
    if let Some(rest) = body.strip_prefix("tr:") {
        let (nonce, rest) = rest.split_once(':')?;
        let (ask, data) = rest.split_once(':')?;
        let value: serde_json::Value = serde_json::from_str(data).ok()?;
        let texts = value
            .get("texts")?
            .as_array()?
            .iter()
            .filter_map(|value| value.as_str().map(ToOwned::to_owned))
            .collect::<Vec<_>>();
        if texts.is_empty() {
            return None;
        }
        return Some(Request::Translate {
            id,
            nonce: nonce.to_string(),
            ask: ask.parse().ok()?,
            claude: value.get("mode").and_then(serde_json::Value::as_str) == Some("claude"),
            texts,
        });
    }
    Some(match body {
        "hide" => Request::Hide,
        "focus:center" => Request::FocusCenter,
        "tab:close" => Request::CloseTab,
        "tab:next" => Request::NextTab,
        "tab:previous" => Request::PreviousTab,
        "tab:new" => Request::NewBlank,
        "help" => Request::Help,
        "history:-1" => Request::History(-1),
        "history:1" => Request::History(1),
        "restart" => Request::Restart,
        "focus:terminal" => Request::FocusTerminal,
        "bookmark" => Request::Bookmark,
        _ => return None,
    })
}

fn select_inner(manager: &mut Manager, index: usize, focus: bool) -> Result<(), String> {
    if manager.tabs.is_empty() {
        manager.active = 0;
        manager.visible = false;
        return Ok(());
    }
    let index = index.min(manager.tabs.len() - 1);
    // 控えならここで起こす。選ばれるまで面を持たないのが控えの約束。
    wake(manager, index)?;
    // 新しい面を先に用意し、旧面はその後で伏せる。逆順では二つの
    // set_visibleの隙間に親のterminalが見える。
    let Some(view) = manager.tabs[index].live() else {
        return Err("the restored tab has no view".into());
    };
    view.set_visible(true)
        .map_err(|error| format!("can't switch browser tabs: {error}"))?;
    for (at, tab) in manager.tabs.iter().enumerate() {
        if at == index {
            continue;
        }
        if let Some(view) = tab.live() {
            view.set_visible(false)
                .map_err(|error| format!("can't switch browser tabs: {error}"))?;
        }
    }
    manager.active = index;
    manager.visible = true;
    if focus && let Some(view) = manager.tabs[index].live() {
        view.focus()
            .map_err(|error| format!("can't focus the browser tab: {error}"))?;
    }
    Ok(())
}

fn snapshot(manager: &Manager) -> Snapshot {
    Snapshot {
        tabs: manager
            .tabs
            .iter()
            .map(|tab| TabInfo {
                id: tab.id,
                title: tab.title.clone(),
                url: tab.url.clone(),
                asleep: tab.asleep(),
            })
            .collect(),
        active: manager.active,
        visible: manager.visible,
    }
}

impl Source {
    fn identity(&self) -> String {
        match self {
            Self::Url(url) | Self::HtmlFile { url, .. } => url.clone(),
            Self::NewTab { serial, .. } => format!("cockpit-new-tab:{serial}"),
        }
    }

    fn title(&self) -> String {
        match self {
            Self::Url(url) => label(url),
            Self::HtmlFile { title, .. } => title.clone(),
            Self::NewTab { .. } => "Browser".to_string(),
        }
    }

    fn translates(&self) -> bool {
        matches!(self, Self::Url(url) if url.starts_with("http://") || url.starts_with("https://"))
    }

    /// 手元のファイル(`file://`)を開いている頁か。ドラフトの HTML・md は
    /// どちらもここへ入る(md は組み立てた HTML を載せるが、身元は元の
    /// `file://` のまま持つ——[`Self::HtmlFile`])。
    fn local_file(&self) -> bool {
        match self {
            Self::Url(url) | Self::HtmlFile { url, .. } => url.starts_with("file://"),
            Self::NewTab { .. } => false,
        }
    }
}

/// ドラフトの頁だけに載せる、リンクを新しいタブへ逃がす仕掛け。
/// 。
/// ドラフトは比較表・出典の一覧が主なので、
/// 1本踏むたびに今見ている表が消えるのは読み方に合わない。
///
/// **外の頁(http/https)には載せない。** サイトの中の遷移まで新しいタブに
/// すると、頁を1つ進むたびにタブが増えて元の頁へ戻れなくなる。判定は
/// 「いま見ている頁が `file://` か」の1点([`Source::local_file`])。
fn local_link_script(source: &Source) -> &'static str {
    if source.local_file() { DRAFT_LINKS } else { "" }
}

/// [`local_link_script`] が載せる本体。
///
/// 拾うのは**押された瞬間**だけ(捕捉相higher で `click`)。頁の中の移動
/// (`#見出し`)と `javascript:` は今までどおり同じ頁で処理する——目次を
/// 踏むたびにタブが増えるのは、ではない。
/// 修飾キー付きの押しは頁より先に器の作法(⌘クリック)が効くので触らない。
const DRAFT_LINKS: &str = r#"
;(function () {
  if (window.__cockpitDraftLinks) { return; }
  window.__cockpitDraftLinks = true;
  function bare(url) { var at = url.indexOf('#'); return at < 0 ? url : url.slice(0, at); }
  document.addEventListener('click', function (event) {
    if (event.defaultPrevented || event.button !== 0) { return; }
    if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) { return; }
    var el = event.target;
    while (el && el !== document) {
      if ((el.tagName || '').toLowerCase() === 'a') { break; }
      el = el.parentNode;
    }
    if (!el || el === document) { return; }
    var attr = el.getAttribute('href');
    if (!attr || /^javascript:/i.test(attr)) { return; }
    var href = el.href;
    if (!href || !/^(https?|file):/i.test(href)) { return; }
    // 同じ頁の中の移動(目次・脚注)はそのまま。
    if (bare(href) === bare(location.href)) { return; }
    event.preventDefault();
    event.stopPropagation();
    try { window.__cockpitSay('tab:open:' + href); } catch (_) {}
  }, true);
})();
"#;

/// 栞1件。`spec` は帳面に書いてある綴りそのもの(削除の鍵)、`url` は
/// WebView が開ける形に直したもの。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Bookmark {
    title: String,
    url: String,
    spec: String,
}

/// 栞を読む(`browser/bookmarks.json`)。壊れた JSON や開けない項目は空として
/// 飛ばす。同じ行き先は1度だけ。
fn bookmarks() -> Vec<Bookmark> {
    let path = armature_core::geo::state_dir().join("browser/bookmarks.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut marks = Vec::new();
    for item in items {
        let Some(spec) = item.get("spec").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let spec = spec.trim();
        if is_internal_page(spec) {
            continue;
        }
        let Some(url) = bookmark_target(spec) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = item
            .get("title")
            .and_then(serde_json::Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .map_or_else(|| label(&url), |title| title.trim().to_string());
        marks.push(Bookmark {
            title,
            url,
            spec: spec.to_string(),
        });
    }
    marks
}

/// 新規タブのローカル頁を組む。
///
/// **置くのは検索欄と栞だけ**。開いている
/// タブの一覧はここから消えた——引数は呼び出し側の形を変えないために残してある。
///
/// 履歴は一覧としては出さず、**打ち始めたときの候補**としてだけ使う
/// (欄の下に浮く)。HTMLへ埋める値は必ずエスケープし、JSへ渡す値は JSON にする。
#[must_use]
#[allow(dead_code)]
pub fn new_tab_html(tabs: &[TabInfo]) -> String {
    new_tab_html_with_nonce(tabs, 0)
}

fn new_tab_html_with_nonce(_tabs: &[TabInfo], nonce: u64) -> String {
    let marks = bookmarks();
    let suggestions = history::suggestions(&marks);
    new_tab_page(&marks, &suggestions, nonce)
}

/// 頁そのものを組む。読み書きを外に出してあるので、栞も候補も手で並べて
/// 検分できる(`<script>` を含む1件で頁が壊れないことを含む)。
fn new_tab_page(marks: &[Bookmark], suggestions: &[String], nonce: u64) -> String {
    let items = marks
        .iter()
        .enumerate()
        .map(|(index, mark)| {
            // 番号は ⌘1–9 と同じ1始まり。押せない行に番号は振らない。
            let number = if index < 9 {
                format!("<span class=\"n\">{}</span>", index + 1)
            } else {
                String::new()
            };
            // 番号は行の外に置かない。外へ出すと選択の面が題だけに掛かり、
            // 番号と面の境が重なって見えた。
            format!(
                "<li><a data-bookmark href=\"{}\">{number}<span class=\"t\">{}</span></a><button class=\"remove\" data-remove-bookmark=\"{}\" aria-label=\"Remove bookmark\">×</button></li>",
                html_escape(&mark.url),
                html_escape(&mark.title),
                html_escape(&mark.spec)
            )
        })
        .collect::<String>();
    let suggestions = script_json(suggestions);
    let hits = history::HITS;
    // 新規タブはコックピットが自分で描く頁なので、色は窓と同じ絵の具から取る
    // ——ここを固定色にすると、絵の具を変えたとき新規タブだけ前の色で残る。
    let bg = css_color(crate::palette::surface_window());
    let raised = css_color(crate::palette::surface_raised());
    let card = css_color(crate::palette::surface_card());
    let active = css_color(crate::palette::surface_active());
    let ink = css_color(crate::palette::text_primary());
    let muted = css_color(crate::palette::text_muted());
    let faint = css_color(crate::palette::text_faint());
    let scheme = if is_dark(crate::palette::surface_window()) {
        "dark"
    } else {
        "light"
    };
    format!(
        r##"<!doctype html>
<html lang="ja"><head><meta charset="utf-8"><title>Browser</title>
<style>
:root{{color-scheme:{scheme};}}
html,body{{margin:0;min-height:100%;background:{bg};color:{ink};overscroll-behavior-y:none;}}
body{{font:16px -apple-system,BlinkMacSystemFont,"Hiragino Sans",sans-serif;padding:48px;box-sizing:border-box;}}
main{{max-width:720px;margin:0 auto;}}
.bar{{position:relative;display:none;}}
form{{display:flex;background:{raised};border-radius:10px;}}
input{{flex:1;padding:12px 14px;border:0;border-radius:10px;outline:0;background:transparent;color:{ink};font:inherit;}}
ul{{list-style:none;padding:22px 0 0;display:grid;gap:2px;}}
li{{position:relative;display:flex;align-items:center;border-radius:8px;}}
a{{display:flex;align-items:center;gap:12px;flex:1;min-width:0;padding:9px 40px 9px 12px;border-radius:8px;color:{ink};text-decoration:none;}}
a:hover,a.selected{{background:{raised};}}
.n{{width:14px;flex:none;color:{faint};font-size:12px;text-align:right;font-variant-numeric:tabular-nums;}}
.t{{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}}
#hits{{position:absolute;top:calc(100% + 6px);left:0;right:0;z-index:2;margin:0;padding:4px;display:block;background:{card};border-radius:9px;max-height:46vh;overflow-y:auto;font-size:13px;box-shadow:0 6px 20px rgba(0,0,0,.35);}}
#hits li{{display:block;padding:5px 12px;border-radius:6px;color:{muted};cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}}
#hits li.on{{background:{active};color:{ink};}}
#hits[hidden]{{display:none;}}
.remove{{position:absolute;right:6px;width:24px;height:24px;padding:0;border:0;border-radius:12px;background:transparent;color:{muted};font:18px/24px sans-serif;opacity:0;cursor:pointer;}}
li:hover .remove,.remove:focus{{opacity:1;}} .remove:hover{{background:{active};color:{ink};}}
</style></head><body><main>
<div class="bar">
<form id="search"><input id="query" name="q" type="search" autocomplete="off" autocapitalize="off" spellcheck="false" autofocus aria-label="Search"></form>
<ul id="hits" hidden></ul>
</div>
<ul>{items}</ul>
<script>(function(){{
var SUGGEST={suggestions};
var HITS={hits};
var form=document.getElementById('search'),input=document.getElementById('query'),hitList=document.getElementById('hits');
var links=Array.prototype.slice.call(document.querySelectorAll('a[data-bookmark]'));
// 開いた直後はどれも選ばない。1件目に面が掛かっていると、検索欄の下に
// 意味の分からない帯が出ているように見えた。
var selected=-1;
function paint(){{links.forEach(function(link,index){{link.classList.toggle('selected',index===selected);}});}}
function move(step){{
  if(!links.length)return false;
  if(selected<0)selected=(step>0)?0:links.length-1;
  else selected=(selected+step+links.length)%links.length;
  paint();
  links[selected].scrollIntoView({{block:'nearest'}});return true;
}}
window.__cockpitNewTab={{move:move}};paint();
// 候補は頁を組むときに焼き込んである。1字打つたびに窓へ問い合わせない。
var hits=[],at=-1,typed='';
function bare(s){{return s.replace(/^[a-z][a-z0-9+.-]*:\/\//,'');}}
function look(text){{
  var q=(text||'').trim().toLowerCase();
  if(!q)return [];
  var head=[],rest=[];
  for(var i=0;i<SUGGEST.length;i++){{
    var s=SUGGEST[i],low=s.toLowerCase();
    if(low.indexOf(q)===0||bare(low).indexOf(q)===0)head.push(s);
    else if(low.indexOf(q)>=0)rest.push(s);
  }}
  return head.concat(rest).slice(0,HITS);
}}
function drawHits(){{
  hitList.textContent='';
  if(!hits.length){{hitList.hidden=true;return;}}
  hits.forEach(function(spec,index){{
    var row=document.createElement('li');
    row.textContent=spec;
    if(index===at)row.className='on';
    row.addEventListener('mousedown',function(e){{e.preventDefault();go(spec);}});
    hitList.appendChild(row);
  }});
  hitList.hidden=false;
}}
function go(value){{
  var text=(value||'').trim();
  if(!text)return;
  try{{window.ipc.postMessage('navigate:{nonce}:'+text);}}catch(_){{}}
}}
form.addEventListener('submit',function(e){{e.preventDefault();go(input.value);}});
input.addEventListener('input',function(){{typed=input.value;hits=look(typed);at=-1;drawHits();}});
input.addEventListener('blur',function(){{hits=[];at=-1;drawHits();}});
input.addEventListener('keydown',function(e){{
  var lower=(e.key||'').toLowerCase();
  // ⌃N / ⌃P は欄に焦点が在るときだけ候補を送る。preventDefault しないと
  // macOS の標準バインド(moveDown:/moveUp:)がキャレットを動かす。
  if(e.ctrlKey&&!e.metaKey&&!e.altKey&&(lower==='n'||lower==='p')){{
    if(!hits.length)return;
    e.preventDefault();e.stopImmediatePropagation();
    at=at+(lower==='n'?1:-1);
    if(at>=hits.length)at=-1;
    if(at<-1)at=hits.length-1;
    input.value=(at<0)?typed:hits[at];
    drawHits();return;
  }}
  if(e.metaKey||e.ctrlKey||e.altKey)return;
  if(e.key!=='Enter')return;
  // 変換中の Return は渡さない。素通しすると変換を決めたつもりが
  // 打ちかけの字で検索に出る。
  if(e.isComposing||e.keyCode===229)return;
  e.preventDefault();e.stopImmediatePropagation();
  go(input.value);
}},true);
Array.prototype.forEach.call(document.querySelectorAll('[data-remove-bookmark]'),function(button){{
  button.addEventListener('click',function(e){{
    e.preventDefault();e.stopPropagation();
    try{{window.ipc.postMessage('bookmark:delete:{nonce}:'+button.dataset.removeBookmark);}}catch(_){{}}
  }});
}});
window.__cockpitBookmarkDeleted=function(spec){{
  var button=Array.prototype.find.call(document.querySelectorAll('[data-remove-bookmark]'),function(item){{return item.dataset.removeBookmark===spec;}});
  if(!button)return;var row=button.closest('li');if(row)row.remove();
  links=Array.prototype.slice.call(document.querySelectorAll('a[data-bookmark]'));
  selected=links.length?Math.min(selected,links.length-1):-1;paint();
}};
links.forEach(function(link){{link.addEventListener('click',function(e){{
  e.preventDefault();
  try{{window.ipc.postMessage('bookmark:open:{nonce}:'+link.href);}}catch(_){{}}
}});}});
window.addEventListener('keydown',function(e){{
  if(e.metaKey||e.ctrlKey||e.altKey||document.activeElement===input)return;
  if(e.key==='Enter'&&selected>=0){{e.preventDefault();try{{window.ipc.postMessage('bookmark:open:{nonce}:'+links[selected].href);}}catch(_){{}}}}
}},true);
}})();</script>
</main></body></html>"##
    )
}

/// JSON を `<script>` の中へ置ける形に潰す。
///
/// 控えも栞も利用者が開いた頁の綴りで、`</script>` を含む1件が混じるとそこで
/// script が閉じ、以降が頁の本文として出る。JSON は `<` を逃がさないので、
/// こちらで潰しておく——読む側の JS には同じ1文字に戻る。
fn script_json(values: &[String]) -> String {
    serde_json::to_string(values)
        .unwrap_or_else(|_| "[]".to_string())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// 打った字を行き先に直す。空なら行き先にならない。
///
/// 判断は上から順に見て、どれにも当たらなければ探しに行く:
///
/// ```text
///   綴りが付いている   https://example.com  → そのまま
///   手元のパス         /Users/x/y.html・~/… → そのまま(`~` は展開する)
///   行き先に見える1語  example.com・localhost:8392 → https:// を足す
///   それ以外           明日の天気           → 探しに行く
/// ```
/// 自分で描く頁の下地。窓と同じ地にして、開いた瞬間の白い閃きを消す。
/// 背景を持たない頁へ敷く明るい紙。既定の黒文字を読めるようにするための下地で、
/// **頁が描き終わってから**敷く(生まれた瞬間に敷くと白く光る)。
const PAPER: (u8, u8, u8, u8) = (0xf7, 0xf5, 0xef, 0xff);

/// 読み込みの段ごとの地の色。替える必要が無ければ `None`。
///
/// 差し替えの合図(`Started` = didCommitNavigation)でいったん窓の色へ戻すので、
/// 頁から頁へ渡るときも白い閃きは挟まらない。紙を敷くのは描き終わった外の
/// サイトだけ——新規タブや資料は色をこちらが決めているので窓の色のままにする。
fn ground_for(event: &wry::PageLoadEvent, url: &str) -> Option<(u8, u8, u8, u8)> {
    match event {
        wry::PageLoadEvent::Started => Some(page_ground()),
        wry::PageLoadEvent::Finished => is_web_url(url).then_some(PAPER),
    }
}

/// タブの地の色を差し替える。開いている最中(`MANAGER` を借りている最中)に
/// 呼ばれることがあるので、借りられなければ何もしない——地は窓の色のまま
/// 残るだけで、白く光ることはない。
fn ground(id: u64, colour: (u8, u8, u8, u8)) {
    MANAGER.with(|manager| {
        let Ok(manager) = manager.try_borrow() else {
            return;
        };
        if let Some(view) = manager.tabs.iter().find(|tab| tab.id == id).and_then(Tab::live) {
            let _ = view.set_background_color(colour);
        }
    });
}

fn page_ground() -> (u8, u8, u8, u8) {
    let colour = crate::palette::surface_window();
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    (byte(colour.r), byte(colour.g), byte(colour.b), 255)
}

/// Iced の絵の具を CSS の字にする。頁へ渡せるのは字だけなので、ここで写す。
fn css_color(color: iced::Color) -> String {
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(color.r),
        byte(color.g),
        byte(color.b)
    )
}

/// 暗い地か。`color-scheme` を決めるためだけの粗い判定。
fn is_dark(color: iced::Color) -> bool {
    // ITU-R BT.601 の重み。頁側の PAGE_SURFACE と同じ読み方に合わせる。
    let brightness = color.r * 0.299 + color.g * 0.587 + color.b * 0.114;
    brightness < 0.588
}

fn typed_target(typed: &str) -> Option<String> {
    let text = typed.trim();
    if text.is_empty() {
        return None;
    }
    // 読めない綴り(空白入りの URL など)は探し物として扱う。
    if (text.contains("://") && loadable(text)) || text.starts_with('/') {
        return Some(text.to_string());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        // `~` を残したまま渡すと、そういう名のディレクトリを探しに行く。
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            return Some(format!("{home}/{rest}"));
        }
    }
    if looks_like_host(text) {
        return Some(format!("https://{text}"));
    }
    Some(format!("{SEARCH}{}", query_escape(text)))
}

/// 行き先に見えるか。**空白が1つでも混じっていれば違う**(打ち間違いではなく
/// 探し物だから)。
fn looks_like_host(text: &str) -> bool {
    if text.chars().any(char::is_whitespace) {
        return false;
    }
    let host = text.split(['/', '?', '#']).next().unwrap_or(text);
    // ポートは落としてから札を数える(`localhost:8392` を1枚の札に見せない)。
    let host = host.split(':').next().unwrap_or(host);
    if host == "localhost" {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|label| label.is_empty()) {
        return false;
    }
    // 末尾の札(TLD)は字だけで2文字以上。`3.14` や `1.2.3` を開きに行かない。
    let tld = labels[labels.len() - 1];
    tld.len() >= 2 && tld.chars().all(|ch| ch.is_ascii_alphabetic())
}

/// 探し物の字を問い合わせに詰める(パーセント符号化)。
fn query_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// 訪れた行き先の控え。**溜める・古いものから落とす・候補として返す**の3つ。
/// 新規タブの欄に打ち始めたときの候補に使う(`browser/history.json`)。
///
/// 並べ方は最近順。頻度順にすると訪れた回数の欄が要り、「回数と新しさの
/// どちらが上か」を毎回決める仕組みになる——まだ誰も困っていないうちから
/// 直しにくいものを建てない。
mod history {
    /// 控えに残す件数。300件なら素の JSON でも数十KBに収まる。
    pub const MAX: usize = 300;
    /// 欄の下に一度に出す候補の数。
    pub const HITS: usize = 8;

    /// 訪れた1件。題は持たない——欄が要るのは行き先の綴りだけ。
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Visit {
        pub spec: String,
        pub at: i64,
    }

    fn path() -> std::path::PathBuf {
        armature_core::geo::state_dir().join("browser/history.json")
    }

    /// 帳面の中身を控えに直す。壊れた要素は黙って飛ばす。
    #[must_use]
    pub fn parse(text: &str) -> Vec<Visit> {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return Vec::new();
        };
        let Some(list) = value.as_array() else {
            return Vec::new();
        };
        list.iter()
            .filter_map(|item| {
                let spec = item.get("spec")?.as_str()?.trim();
                if spec.is_empty() {
                    return None;
                }
                Some(Visit {
                    spec: spec.to_string(),
                    at: item
                        .get("at")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                })
            })
            .take(MAX)
            .collect()
    }

    /// 読む。読めなければ空——控えは利用者が手で消し得るもので、そのたびに
    /// 窓が起きなくなるのは割に合わない。
    #[must_use]
    pub fn load() -> Vec<Visit> {
        std::fs::read_to_string(path()).map_or_else(|_| Vec::new(), |text| parse(&text))
    }

    /// 1件足す。**同じ行き先は2度持たない**——先頭へ動かして時刻だけ新しくし、
    /// 溢れたぶんはいちばん古いものから落とす。
    #[must_use]
    pub fn add(mut visits: Vec<Visit>, spec: &str, at: i64) -> Vec<Visit> {
        let spec = spec.trim();
        if spec.is_empty() {
            return visits;
        }
        visits.retain(|visit| visit.spec != spec);
        visits.insert(
            0,
            Visit {
                spec: spec.to_string(),
                at,
            },
        );
        visits.truncate(MAX);
        visits
    }

    /// 1件覚える(読んで足して書く)。書けなくても窓は動かし続ける。
    pub fn remember(spec: &str) {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0));
        let visits = add(load(), spec, at);
        let file = path();
        let Some(parent) = file.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let list: Vec<serde_json::Value> = visits
            .iter()
            .map(|visit| serde_json::json!({ "spec": visit.spec, "at": visit.at }))
            .collect();
        if let Ok(text) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::write(&file, text);
        }
    }

    /// 頁へ焼き込む候補。控えが先、栞が後(控えに在るものは二重に出さない)。
    #[must_use]
    pub fn suggestions(marks: &[super::Bookmark]) -> Vec<String> {
        merge(&load(), marks)
    }

    /// 控えと栞を1本の並びに畳む(検分できるように分けてある)。
    #[must_use]
    pub fn merge(visits: &[Visit], marks: &[super::Bookmark]) -> Vec<String> {
        let mut out: Vec<String> = visits.iter().map(|visit| visit.spec.clone()).collect();
        for mark in marks {
            if !out.iter().any(|spec| spec == &mark.url) {
                out.push(mark.url.clone());
            }
        }
        out.truncate(MAX);
        out
    }
}

/// 新規タブ頁を `Source` として返す便利な公開口。
#[must_use]
#[allow(dead_code)]
pub fn new_tab_source(tabs: &[TabInfo]) -> Source {
    let serial = NEXT_NEW_TAB.fetch_add(1, Ordering::Relaxed);
    Source::NewTab {
        serial,
        html: new_tab_html_with_nonce(tabs, serial),
    }
}

fn is_web_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// 外から渡された URL(自作パネルの `Host::open_url`)を検める。通すのは宿の名を持つ
/// http/https の頁で、wry が読める綴り([`loadable`])だけ。`file:`・`javascript:`・
/// 手元のアプリを起こす独自の scheme は断る。scheme の大文字は小文字に揃える。
pub(crate) fn checked_web_url(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let url = format!("{scheme}://{rest}");
    (!host_of(&url).is_empty() && loadable(&url)).then_some(url)
}

fn is_ad_url(url: &str) -> bool {
    const HOSTS: &[&str] = &[
        "doubleclick.net",
        "googlesyndication.com",
        "googleadservices.com",
        "amazon-adsystem.com",
        "adnxs.com",
        "criteo.com",
        "criteo.net",
        "taboola.com",
        "outbrain.com",
    ];
    let host = host_of(url);
    HOSTS
        .iter()
        .any(|blocked| host == *blocked || host.ends_with(&format!(".{blocked}")))
}

/// URL から宿の名だけを取り出す(小文字・港と末尾の点は落とす)。
fn host_of(url: &str) -> String {
    let Some((_, tail)) = url.split_once("://") else {
        return String::new();
    };
    let authority = tail.split(['/', '?', '#']).next().unwrap_or_default();
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn is_persistable_url(url: &str) -> bool {
    is_web_url(url) || url.starts_with("file:///")
}

/// 器が自分で置いた内部頁(新規タブの空白頁など `state_dir/browser/` 配下)か。
/// 栞には**読む側でも入れる側でも**入れない。
fn is_internal_page(spec: &str) -> bool {
    let root = armature_core::geo::state_dir().join("browser");
    let root = root.to_string_lossy();
    let path = spec.strip_prefix("file://").unwrap_or(spec);
    !root.is_empty() && path.starts_with(root.as_ref())
}

/// 栞の絶対パスを WebView が受け取れる file URL へ変換する。
///
/// 存在しないパスは一覧へ出さない。`#` や空白を URL の断片・区切りとして
/// 解釈させないため、最低限の文字だけ percent-escape する。
#[allow(dead_code)]
fn bookmark_target(spec: &str) -> Option<String> {
    if is_web_url(spec) || spec.starts_with("file:///") {
        return Some(spec.to_string());
    }
    let path = Path::new(spec);
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    Some(file_url(path))
}

#[allow(dead_code)]
fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for ch in path.to_string_lossy().chars() {
        match ch {
            ' ' => url.push_str("%20"),
            '#' => url.push_str("%23"),
            '?' => url.push_str("%3F"),
            other => url.push(other),
        }
    }
    url
}

/// 手元の HTML や Markdown を、外部ヘルパーを挟まず中央ブラウザへ渡す。
#[must_use]
pub fn source_for_path(path: &Path) -> Source {
    const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if browser_native_file(extension) {
        return Source::Url(file_url(path));
    }
    let Ok(bytes) = read_prefix(path, MAX_TEXT_BYTES + 1) else {
        return Source::Url(file_url(path));
    };
    let truncated = bytes.len() > MAX_TEXT_BYTES;
    let visible = &bytes[..bytes.len().min(MAX_TEXT_BYTES)];
    if extension.eq_ignore_ascii_case("md") {
        let source = String::from_utf8_lossy(visible);
        let title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        return Source::HtmlFile {
            html: crate::md::to_html(&source, &title),
            title,
            url: file_url(path),
        };
    }
    if text_file(extension, &bytes) {
        let source = String::from_utf8_lossy(visible);
        let title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        return Source::HtmlFile {
            html: plain_text_html(&source, &title, truncated),
            title,
            url: file_url(path),
        };
    }
    Source::Url(file_url(path))
}

fn read_prefix(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn browser_native_file(extension: &str) -> bool {
    const EXTENSIONS: [&str; 23] = [
        "htm", "html", "pdf", "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "bmp", "tif",
        "tiff", "avif", "mp3", "m4a", "wav", "aac", "mp4", "m4v", "mov", "webm", "ogg",
    ];
    EXTENSIONS
        .iter()
        .any(|known| extension.eq_ignore_ascii_case(known))
}

fn text_file(extension: &str, bytes: &[u8]) -> bool {
    const TEXT_EXTENSIONS: [&str; 51] = [
        "conf",
        "config",
        "cfg",
        "ini",
        "env",
        "toml",
        "yaml",
        "yml",
        "json",
        "jsonl",
        "txt",
        "log",
        "rs",
        "swift",
        "py",
        "rb",
        "go",
        "java",
        "kt",
        "kts",
        "js",
        "mjs",
        "cjs",
        "ts",
        "tsx",
        "jsx",
        "css",
        "scss",
        "sass",
        "less",
        "sh",
        "bash",
        "zsh",
        "fish",
        "ps1",
        "sql",
        "csv",
        "tsv",
        "xml",
        "plist",
        "properties",
        "gradle",
        "c",
        "h",
        "cc",
        "cpp",
        "cxx",
        "hpp",
        "hxx",
        "m",
        "mm",
    ];
    if TEXT_EXTENSIONS
        .iter()
        .any(|known| extension.eq_ignore_ascii_case(known))
    {
        return true;
    }
    if bytes.contains(&0) {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        // 上限ちょうどでUTF-8の一文字だけが途中になった場合も本文として扱う。
        Err(error) => error.error_len().is_none() && error.valid_up_to() > 0,
    }
}

fn plain_text_html(source: &str, title: &str, truncated: bool) -> String {
    let notice = if truncated {
        tr!(
            "<p class=notice>Showing the first 2 MiB only</p>",
            "<p class=notice>先頭2 MiBだけを表示</p>",
        )
    } else {
        ""
    };
    format!(
        r#"<!doctype html><html lang="ja"><head><meta charset="utf-8"><title>{}</title>
<style>:root{{color-scheme:dark}}html,body{{background:#303446;color:#c6d0f5}}body{{margin:0;padding:24px}}h1{{font:600 14px -apple-system,sans-serif;color:#e5c890;margin:0 0 16px}}.notice{{font:12px -apple-system,sans-serif;color:#eebebe}}pre{{font:13px/1.65 "Moralerspace Argon Cockpit",ui-monospace,monospace;white-space:pre-wrap;overflow-wrap:anywhere;margin:0}}</style></head><body><h1>{}</h1>{notice}<pre>{}</pre></body></html>"#,
        html_escape(title),
        html_escape(title),
        html_escape(source),
    )
}

/// 頁の外から届いた URL(⌘クリック・栞・保存したタブ)を、開く形に直す。
///
/// `file://` の md とテキストは組み立てた HTML にする。素のまま WebView へ渡すと
/// 文字コードの宣言が無いので UTF-8 の日本語が全文化けた。
/// それ以外は届いた綴りのまま——HTML の身元(`%` の有無)を今と変えない。
#[must_use]
pub fn source_for_url(url: &str) -> Source {
    match source_for_saved_url(url) {
        html @ Source::HtmlFile { .. } => html,
        _ => Source::Url(url.to_string()),
    }
}

fn source_for_saved_url(url: &str) -> Source {
    let Some(path) = file_path(url) else {
        return Source::Url(url.to_string());
    };
    source_for_path(&path)
}

fn file_path(url: &str) -> Option<std::path::PathBuf> {
    let path = url.strip_prefix("file://")?;
    Some(std::path::PathBuf::from(percent_decoded(path)))
}

pub(crate) fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn label(url: &str) -> String {
    // file:// はホスト名が空で、素朴に切ると先頭の `file:` だけが残る。
    // 手元の頁も読める名前が要るので、ファイル名を表示名にする。
    if let Some(path) = file_path(url) {
        return path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Browser".to_string());
    }
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("Browser")
        .to_string()
}

fn pages_path() -> std::path::PathBuf {
    armature_core::geo::state_dir().join("armature-pages.tsv")
}

fn rect(bounds: Bounds) -> WryRect {
    WryRect {
        position: LogicalPosition::new(bounds.x, bounds.y).into(),
        size: LogicalSize::new(bounds.width, bounds.height).into(),
    }
}

fn raise(webview: &WebView) {
    use objc2_app_kit::NSView;
    use wry::WebViewExtMacOS;

    let web = webview.webview();
    // 二本指のピンチで拡大縮小できるようにする。WKWebView の既定は「不可」で、
    // 切ったままだと普通のブラウザなら効く操作がこのパネルでだけ死ぬ。頁の拡大率(setPageZoom)とは別の層で、こちらはパネルの中を
    // 虫眼鏡で覗く動き——版面の折り返しは変わらない。
    unsafe {
        let _: () = objc2::msg_send![&*web, setAllowsMagnification: true];
    }
    let view: &NSView = &web;
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setZPosition(1.0);
        // 角丸はパネルのカードと同じ値を使う。**ここを固定値にすると、角を立てた
        // ときにブラウザだけ丸いまま残る**。
        layer.setCornerRadius(f64::from(crate::palette::radius_card()));
        layer.setMasksToBounds(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_tabs_are_parsed_as_asleep() {
        // 復元は札を並べるだけ。起きるのは選ばれてから(2026-09-02)。
        let saved = parse_saved("一枚目\thttps://a.invalid/\t0\n二枚目\thttps://b.invalid/\t1\n");
        assert_eq!(saved.tabs.len(), 2);
        assert!(saved.tabs.iter().all(|tab| tab.asleep));
        assert_eq!(saved.active, 1);
    }

    #[test]
    fn asleep_tabs_need_a_window_to_wake() {
        let mut manager = Manager::default();
        manager.tabs.push(Tab {
            id: 1,
            title: "控え".into(),
            url: "https://asleep.invalid/".into(),
            new_tab_nonce: None,
            surface: Surface::Asleep,
        });
        assert!(snapshot(&manager).tabs[0].asleep);
        // 窓の手掛かりが無いうちは起こせない——面の無いまま黙って選ばせない。
        assert!(wake(&mut manager, 0).is_err());
        // 居ない札は何もしない。
        assert!(wake(&mut manager, 9).is_ok());
    }

    #[test]
    fn local_pages_are_labelled_by_file_name() {
        assert_eq!(
            label("file:///Users/someone/notes/調査%20メモ.html"),
            "調査 メモ.html"
        );
        assert_eq!(label("file:///"), "Browser");
        assert_eq!(label("https://example.com/a/b"), "example.com");
    }

    #[test]
    fn browser_messages_keep_the_tab_that_sent_them() {
        assert_eq!(
            parse(7, "title:An article"),
            Some(Request::Title {
                id: 7,
                title: "An article".into()
            })
        );
        let request = parse(9, r#"tr:page-a:3:{"mode":"auto","texts":["Hello"]}"#);
        assert_eq!(
            request,
            Some(Request::Translate {
                id: 9,
                nonce: "page-a".into(),
                ask: 3,
                claude: false,
                texts: vec!["Hello".into()]
            })
        );
    }

    #[test]
    fn the_data_store_identity_is_stable_for_cookie_persistence() {
        assert_eq!(&DATA_STORE[..8], b"armature");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_browser_shortcuts_work_without_a_dom_key_event() {
        let command = NativeModifiers {
            command: true,
            ..NativeModifiers::default()
        };
        assert_eq!(native_request("w", 13, command), Some(Request::CloseTab));
        assert_eq!(native_request("b", 11, command), Some(Request::Hide));
        assert_eq!(native_request("h", 4, command), Some(Request::FocusCenter));
        // ⌘L は頁を見ている間はアドレス欄、⌘R は読み直し。
        assert_eq!(native_request("l", 37, command), Some(Request::FocusAddress));
        assert_eq!(native_request("r", 15, command), Some(Request::Reload));
        assert_eq!(
            native_request("2", 19, command),
            Some(Request::SelectTab(1))
        );
        // 器のキーは頁の上でも効く(WebView に渡すと黙って捨てられる)。
        assert_eq!(native_request("j", 38, command), Some(Request::NewShell));
        assert_eq!(native_request(",", 43, command), Some(Request::Settings));
        assert_eq!(native_request("、", 43, command), Some(Request::Settings));
        // ⇧⌘T は端末の側と同じく閉じた Claude タブを戻す(シェルではない)。
        assert_eq!(
            native_request(
                "t",
                17,
                NativeModifiers {
                    shift: true,
                    ..command
                }
            ),
            Some(Request::Reopen)
        );

        let control = NativeModifiers {
            control: true,
            ..NativeModifiers::default()
        };
        assert_eq!(native_request("\t", 48, control), Some(Request::NextTab));
        assert_eq!(
            native_request(
                "\t",
                48,
                NativeModifiers {
                    shift: true,
                    ..control
                }
            ),
            Some(Request::PreviousTab)
        );
        assert!(
            !KEYS.contains("event.ctrlKey && key === 'Tab'"),
            "native monitorとDOMの両方でタブを送ると2枚飛ばす"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn command_brackets_walk_the_history_of_the_showing_tab() {
        let command = NativeModifiers {
            command: true,
            ..NativeModifiers::default()
        };
        assert_eq!(native_history_step("[", command), Some(-1));
        assert_eq!(native_history_step("]", command), Some(1));
        assert_eq!(
            native_history_step(
                "[",
                NativeModifiers {
                    shift: true,
                    ..command
                }
            ),
            None
        );
        assert_eq!(native_history_step("[", NativeModifiers::default()), None);
        assert_eq!(native_history_step("b", command), None);
        // 履歴のキーは`Request`にしない——lib.rsの分岐を増やさずに済ませる。
        assert_eq!(native_request("[", 33, command), None);
        assert_eq!(native_request("]", 30, command), None);
    }

    #[test]
    fn only_pages_the_app_built_count_as_inside() {
        let main = true;
        assert!(!from_outside("about:blank", main, Some("about:blank")));
        assert!(!from_outside("file:///tmp/draft.html", main, Some("file:///tmp/draft.html")));
        // 外の頁が about:blank の枠から言っても、載っている頁で外と分かる。
        assert!(from_outside("about:blank", main, Some("https://evil.example/")));
        assert!(from_outside("https://ads.example/frame", main, Some("about:blank")));
        assert!(from_outside("https://evil.example/", main, Some("https://evil.example/")));
        assert!(from_outside("about:blank", main, Some("blob:https://evil.example/1")));
        assert!(from_outside("about:blank", main, None));
    }

    #[test]
    fn a_panel_may_open_web_pages_only() {
        assert_eq!(
            checked_web_url(" https://example.com/a?b=c "),
            Some("https://example.com/a?b=c".to_string())
        );
        assert_eq!(checked_web_url("HTTP://example.com"), Some("http://example.com".to_string()));
        for refused in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "x-apple.systempreferences:com.apple.preference.security",
            "ssh://example.com",
            "https://",
            "http://exa mple.com",
            "-a Calculator",
            "",
        ] {
            assert_eq!(checked_web_url(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_frame_inside_an_app_page_speaks_as_an_outside_page() {
        // 手元の HTML(器の頁)に埋め込まれた外の iframe が、自分の中に about:blank や
        // about:srcdoc の枠を立てて言う。送り主の URL も載っている頁も「内」に見えるが、
        // 主フレームではないので外として仕分ける。
        let draft = Some("file:///Users/someone/notes/draft.html");
        for sender in ["about:blank", "about:srcdoc", "https://evil.example/embed"] {
            assert!(from_outside(sender, false, draft), "{sender}");
        }
        assert!(from_outside("about:blank", false, Some("about:blank")));
        // 主フレームの器の頁だけが内。
        assert!(!from_outside("file:///Users/someone/notes/draft.html", true, draft));
    }

    #[test]
    fn a_frame_cannot_even_rename_the_tab() {
        // 外の頁の主フレームなら題は受けるが、iframe からは題も受けない。
        let queued = |id: u64| {
            REQUESTS.lock().unwrap().iter().any(
                |request| matches!(request, Request::Title { id: at, .. } if *at == id),
            )
        };
        receive(9_870_001, "title:spoofed", "https://ads.example/frame", false);
        receive(9_870_001, "title:spoofed", "about:blank", false);
        assert!(!queued(9_870_001));
        receive(9_870_002, "title:Example", "https://example.com/", true);
        assert!(queued(9_870_002));
        REQUESTS
            .lock()
            .unwrap()
            .retain(|request| !matches!(request, Request::Title { id: 9_870_002, .. }));
    }

    #[test]
    fn outside_pages_get_titles_and_automatic_translation_only() {
        let translate = |id, claude| Request::Translate {
            id,
            nonce: "n".into(),
            ask: 1,
            claude,
            texts: vec!["Hello".into()],
        };
        let title = Request::Title { id: 1, title: "t".into() };
        assert_eq!(outside_request(title.clone()), Some(title));
        assert_eq!(outside_request(Request::History(-1)), Some(Request::History(-1)));
        assert_eq!(outside_request(translate(1, false)), Some(translate(1, false)));
        for privileged in [
            Request::Restart,
            Request::CloseTab,
            Request::Bookmark,
            Request::SelectTab(0),
            Request::NewTab("file:///etc/hosts".into()),
        ] {
            assert_eq!(outside_request(privileged.clone()), None, "{privileged:?}");
        }
        // 精訳は ⇧T を押したタブだけ、束の数まで。
        *CLAUDE_ARM.lock().unwrap() = None;
        assert_eq!(outside_request(translate(7, true)), None);
        note_key_down(Some(7), true);
        assert_eq!(outside_request(translate(8, true)), None, "押していないタブ");
        for _ in 0..CLAUDE_ARM_BATCHES {
            assert_eq!(outside_request(translate(7, true)), Some(translate(7, true)));
        }
        assert_eq!(outside_request(translate(7, true)), None, "1 回の ⇧T で頁 1 枚まで");
        // 外の頁の新しいタブは、鍵を押した直後の web の頁を 1 枚だけ。
        let web = Request::NewTab("https://example.com/".into());
        note_key_down(None, false);
        assert_eq!(outside_request(web.clone()), Some(web.clone()));
        assert_eq!(outside_request(web), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_shift_t_arms_claude_translation() {
        let shift = NativeModifiers { shift: true, ..NativeModifiers::default() };
        assert!(asks_for_claude("T", shift));
        assert!(!asks_for_claude("t", NativeModifiers::default()));
        assert!(!asks_for_claude("T", NativeModifiers { command: true, ..shift }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_browser_monitor_leaves_terminal_keys_untouched_when_hidden() {
        assert_eq!(
            visible_native_request(
                false,
                "w",
                13,
                NativeModifiers {
                    command: true,
                    ..NativeModifiers::default()
                }
            ),
            None
        );
    }

    #[test]
    fn page_scripts_ignore_embedded_frames() {
        assert!(KEYS.contains("window.top !== window"));
        assert!(HINTS.contains("window.__cockpitHints"));
        assert!(TRANSLATION.contains("window.top !== window"));
    }

    #[test]
    fn transparent_page_surface_follows_the_site_ink_brightness() {
        assert!(PAGE_SURFACE.contains("brightness(ink) < 150"));
        assert!(PAGE_SURFACE.contains("'#f7f5ef' : '#303446'"));
        assert!(PAGE_SURFACE.contains("['article p','main p'"));
        assert!(PAGE_SURFACE.contains("surface, 'important'"));
        assert!(PAGE_SURFACE.contains("document.body.style.setProperty('background-color'"));
        assert!(PAGE_SURFACE.contains("backgroundImage !== 'none'"));
    }

    /// 新規タブはコックピットが描く頁。色は窓と同じ絵の具から来る
    /// ——固定色が残っていると、絵の具を変えたときここだけ前の色になる。
    #[test]
    fn the_paper_ground_is_laid_only_after_an_outside_page_has_painted() {
        // 生まれた瞬間と差し替えの合図では窓と同じ色。ここに紙を置くと白く光る。
        assert_eq!(
            ground_for(&wry::PageLoadEvent::Started, "https://example.com"),
            Some(page_ground())
        );
        // 描き終わった外のサイトにだけ紙を敷く。
        assert_eq!(
            ground_for(&wry::PageLoadEvent::Finished, "https://example.com"),
            Some(PAPER)
        );
        // 自分で描く頁(資料・新規タブ)は最後まで窓の色。
        assert_eq!(
            ground_for(&wry::PageLoadEvent::Finished, "file:///Users/x/a.html"),
            None
        );
        assert_eq!(ground_for(&wry::PageLoadEvent::Finished, "about:blank"), None);
    }

    #[test]
    fn the_new_tab_takes_its_colours_from_the_palette() {
        let page = new_tab_html(&[]);
        let window = css_color(crate::palette::surface_window());

        let ink = css_color(crate::palette::text_primary());
        let raised = css_color(crate::palette::surface_raised());

        // 窓の色と、いま焼いた頁の色が同じ絵の具から来ていること。
        // 値そのものを書かない——絵の具を差し替えたらこの試験ごと動く。
        assert!(page.contains(&format!("background:{window};color:{ink};")));
        assert!(page.contains(&format!("form{{display:flex;background:{raised};")));
        assert!(page.contains("color-scheme:"));
    }

    #[test]
    fn colours_become_six_digit_css() {
        assert_eq!(css_color(iced::Color::BLACK), "#000000");
        assert_eq!(css_color(iced::Color::WHITE), "#ffffff");
        assert!(is_dark(iced::Color::BLACK));
        assert!(!is_dark(iced::Color::WHITE));
        assert!(!PAGE_SURFACE.contains("background-color:#303446 !important"));
        assert!(PAGE_SURFACE.contains("overscroll-behavior-y:none"));
        assert!(PAGE_SURFACE.contains("__cockpitSurface"));
    }

    /// 訳文は textContent の丸ごと置換なので、`<style>`/`<script>` を抱えた塊を
    /// 掴むと頁の綴りごと訳文に化ける。掴む前に外していることと、既に日本語の塊を
    /// 英→日に投げないことを見張る(2026-08-19 Amazon 商品頁の崩れ)。
    #[test]
    fn translation_leaves_blocks_that_carry_page_machinery_alone() {
        assert!(TRANSLATION.contains("var FRAGILE = 'style,script,noscript,template,link,meta,"));
        assert!(TRANSLATION.contains("return !!(el.querySelector && el.querySelector(FRAGILE));"));
        assert!(TRANSLATION.contains("function foreign(text) {"));
        assert!(TRANSLATION.contains("if (!/[A-Za-z]{2}/.test(text)) return false;"));
        assert!(TRANSLATION.contains("\\u3040-\\u30ff"));
        assert!(TRANSLATION.contains("|| !foreign(text)) continue;"));
        assert!(TRANSLATION.contains("!skipped(block) && foreign(text)"));
        // 素の英字判定だけで掴む口は残っていない。
        assert!(!TRANSLATION.contains("!/[A-Za-z]{2}/.test(text)) continue"));
    }

    #[test]
    fn delayed_translation_observer_is_bounded_and_skips_finished_blocks() {
        assert!(TRANSLATION.contains("AUTO_RUN_LIMIT = 16"));
        assert!(TRANSLATION.contains("EMPTY_RUN_LIMIT = 8"));
        assert!(TRANSLATION.contains("AUTO_UNTIL = Date.now() + 120000"));
        assert!(TRANSLATION.contains("!block.__trDone"));
        assert!(TRANSLATION.contains("record.addedNodes"));
        assert!(TRANSLATION.contains("scheduleAuto(450)"));
    }

    #[test]
    /// ドラフト(`file://`)の中のリンクだけ新しいタブへ逃がす。外のサイトへは
    /// 載せない——サイト内の遷移までタブにすると、1頁進むたびにタブが増える。
    fn only_local_draft_pages_route_their_links_to_a_new_tab() {
        let draft = Source::Url("file:///Users/someone/notes/a.html".into());
        let markdown = Source::HtmlFile {
            title: "a.md".into(),
            html: "<p>x</p>".into(),
            url: "file:///Users/someone/notes/a.md".into(),
        };
        let site = Source::Url("https://example.com/".into());
        let blank = Source::NewTab {
            serial: 1,
            html: "<p>x</p>".into(),
        };

        assert_eq!(local_link_script(&draft), DRAFT_LINKS);
        assert_eq!(local_link_script(&markdown), DRAFT_LINKS);
        assert_eq!(local_link_script(&site), "");
        assert_eq!(local_link_script(&blank), "");

        // 逃がし先は既存の口。押した href をそのまま新しいタブへ渡す。
        assert!(DRAFT_LINKS.contains("tab:open:"));
        assert_eq!(
            parse(7, "tab:open:file:///Users/someone/notes/b.html"),
            Some(Request::NewTab(
                "file:///Users/someone/notes/b.html".into()
            ))
        );
        // 頁の中の移動(目次)と javascript: は同じ頁のまま。
        assert!(DRAFT_LINKS.contains("bare(href) === bare(location.href)"));
        assert!(DRAFT_LINKS.contains("javascript:"));
    }

    #[test]
    /// **頁に載せる仕掛けは、どれも [`SAY`] の口を通す。**
    ///
    /// 利用者症状2026-09-06「ドラフトの中の YouTube のリンクを押しても何も
    /// 起きない」。`file://` の頁からの伝言が wry の受け口で捨てられていた。
    /// 受け口は自前([`ipc`])になり、主フレームから直に投げれば届く。
    /// 口を1つに絞っておけば、伝言の出し方を変えるときに直す所も1つで済む。
    fn page_scripts_speak_through_one_door_from_the_main_frame() {
        assert!(SAY.contains("window.__cockpitSay"));
        assert!(SAY.contains("window.ipc.postMessage"));
        // 隠しフレームの回り道は畳んだ。iframe からの伝言は外として弾かれる。
        assert!(!SAY.contains("createElement('iframe')"));

        // 頁に載せる仕掛けは、どれも直の口を叩かない。
        for (name, script) in [
            ("KEYS", KEYS),
            ("SWIPE", SWIPE),
            ("HINTS", HINTS),
            ("PAGE_SURFACE", PAGE_SURFACE),
            ("AD_BLOCK", AD_BLOCK),
            ("HUSH", HUSH),
            ("TRANSLATION", TRANSLATION),
            ("DRAFT_LINKS", DRAFT_LINKS),
        ] {
            assert!(
                !script.contains("window.ipc.postMessage"),
                "{name} は __cockpitSay を通すこと"
            );
        }
        assert!(KEYS.contains("window.__cockpitSay"));
        assert!(DRAFT_LINKS.contains("window.__cockpitSay('tab:open:' + href)"));

        // 口はいつでも先頭。ドラフトも、外のサイトも、自前の頁も。
        let draft = Source::Url("file:///Users/someone/notes/a.html".into());
        let site = Source::Url("https://example.com/".into());
        let blank = Source::NewTab {
            serial: 1,
            html: "<p>x</p>".into(),
        };
        for source in [&draft, &site, &blank] {
            assert!(page_script(source, false).starts_with(SAY));
        }
        assert!(page_script(&draft, false).contains(DRAFT_LINKS));
    }

    #[test]
    fn hint_script_routes_shift_and_blank_targets_to_new_tabs() {
        assert!(HINTS.contains("event.key === 'f'"));
        assert!(HINTS.contains("event.shiftKey"));
        assert!(HINTS.contains("target"));
        assert!(HINTS.contains("tab:open:"));
        assert!(HINTS.contains("location.href = href"));
        assert!(HINTS.contains("event.key === 'Escape'"));
    }

    #[test]
    fn scroll_keys_ease_instead_of_jumping() {
        // 瞬間移動の scrollBy へ戻していないこと。j/k は目標だけを積み、
        // 実際の位置は毎フレーム寄せる——ここが崩れると頁がまた飛ぶ。
        assert!(!KEYS.contains("scrollBy("));
        assert!(KEYS.contains("window.__cockpitScroll.by(STEP)"));
        assert!(KEYS.contains("window.__cockpitScroll.by(-STEP)"));
        assert!(KEYS.contains("requestAnimationFrame(step)"));
        // サイト側の scroll-behavior:smooth に二重で乗らない。
        assert!(KEYS.contains("behavior: 'instant'"));
        // **位置は経過時間の関数で出す。**コマごとに残りの割合を詰める作りへ戻すと、
        // 重い頁でコマが落ちたぶんの距離が置き去りになり、溜めてから一度に飛ぶ
        // (2026-08-30 実測: 6.7fps の頁で 1コマ 463px)。
        assert!(KEYS.contains("Math.exp(-(now - t0) / TAU)"));
        assert!(!KEYS.contains("Math.pow(0.0018"));
        // 1コマの空振りで追従を降りない(重い頁では丸めや遅れで普通に起きる)。
        assert!(KEYS.contains("++stalls >= 4"));
        // 別の手で動かし始めたら目標を捨てる。
        assert!(KEYS.contains("'wheel'"));
    }

    #[test]
    fn hint_script_measures_only_what_is_actually_visible() {
        // 隠れている・切られている要素へ札を出すと狙いが外れる。
        assert!(HINTS.contains("elementFromPoint"));
        assert!(HINTS.contains("overflow"));
        assert!(HINTS.contains("pointerEvents"));
        // 札は画面の並び順に振り、重なったら逃がす。
        assert!(HINTS.contains("targets.sort"));
        assert!(HINTS.contains("placed.push"));
        // 流している最中に測らない——止めて1コマ待つ。
        assert!(HINTS.contains("window.__cockpitScroll.stop()"));
        assert!(HINTS.contains("requestAnimationFrame(show)"));
        // href を持たない押しものはマウス列を合成する。
        assert!(HINTS.contains("pointerdown"));
    }

    #[test]
    fn ad_block_targets_only_explicit_slots_and_known_hosts() {
        assert!(AD_BLOCK.contains("[data-ad-slot]"));
        assert!(AD_BLOCK.contains("MutationObserver"));
        assert!(is_ad_url(
            "https://pagead2.googlesyndication.com/pagead/js/ads.js"
        ));
        assert!(is_ad_url("https://ads.example.doubleclick.net/popup"));
        assert!(!is_ad_url("https://example.com/article/ad-design"));
        assert!(!is_ad_url("file:///tmp/ad.html"));
    }

    #[test]
    fn internal_pages_are_never_bookmarks() {
        // 。
        let blank = armature_core::geo::state_dir()
            .join("browser/blank.html")
            .to_string_lossy()
            .into_owned();
        assert!(is_internal_page(&blank));
        assert!(is_internal_page(&format!("file://{blank}")));
        assert!(!is_internal_page("https://example.com/blank.html"));
        assert!(!is_internal_page("/Users/someone/notes/x.html"));
    }

    #[test]
    fn the_new_tab_opens_without_a_selected_row_and_keeps_the_number_inside_it() {
        // 。
        let marks = [sample_bookmark("栞の題", "https://example.com/one")];
        let html = new_tab_page(&marks, &[], 3);
        assert!(html.contains(
            "<a data-bookmark href=\"https://example.com/one\"><span class=\"n\">1</span>"
        ));
        assert!(html.contains("var selected=-1;"));
        assert!(!html.contains("var selected=links.length?0:-1;"));
        assert!(!html.contains("#e5c890"));
    }

    fn sample_bookmark(title: &str, url: &str) -> Bookmark {
        Bookmark {
            title: title.to_string(),
            url: url.to_string(),
            spec: url.to_string(),
        }
    }

    #[test]
    fn the_new_tab_shows_only_the_search_box_and_bookmarks() {
        // 。
        let html = new_tab_html(&[TabInfo {
            id: 1,
            title: "開いているタブの題".into(),
            url: "https://tab-only.invalid/opened".into(),
            asleep: false,
        }]);
        assert!(
            !html.contains("https://tab-only.invalid/opened"),
            "現在のタブが新規タブに出ている"
        );
        assert!(!html.contains("開いているタブの題"));
        assert!(html.contains("id=\"query\""));
        assert!(html.contains("data-bookmark"));
        assert!(html.contains("window.__cockpitNewTab"));
        assert!(html.contains("classList.toggle('selected'"));
        assert!(html.contains("bookmark:open:0:"));
        assert!(!html.contains("location.href=link.href"));
        assert!(html.contains("overscroll-behavior-y:none"));
        assert!(!html.contains("<h1>新しいタブ</h1>"));
        assert!(!html.contains("placeholder=\"検索\""));
        let source = new_tab_source(&[]);
        assert_eq!(source.title(), "Browser");
        assert!(matches!(source, Source::NewTab { .. }));
    }

    #[test]
    fn the_new_tab_escapes_every_value_it_bakes_into_the_page() {
        let marks = vec![sample_bookmark(
            "栞 <script>alert(1)</script>",
            "https://example.com/?q=\"x\"&y=1",
        )];
        let suggestions = vec!["https://example.com/</script><b>".to_string()];
        let html = new_tab_page(&marks, &suggestions, 7);
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("q=&quot;x&quot;&amp;y=1"));
        assert!(!html.contains("<script>alert(1)"));
        // 候補は JSON だが、`</script>` を含む1件で頁が閉じないよう潰してある。
        assert!(html.contains("\\u003c/script\\u003e"));
        assert_eq!(html.matches("</script>").count(), 1);
    }

    #[test]
    fn the_new_tab_numbers_the_first_nine_bookmarks_for_the_command_keys() {
        let marks: Vec<Bookmark> = (0..11)
            .map(|index| {
                sample_bookmark(&format!("栞{index}"), &format!("https://n{index}.example/"))
            })
            .collect();
        let html = new_tab_page(&marks, &[], 1);
        assert!(html.contains("<span class=\"n\">1</span>"));
        assert!(html.contains("<span class=\"n\">9</span>"));
        assert!(!html.contains("<span class=\"n\">10</span>"));
    }

    #[test]
    fn typed_addresses_open_and_typed_words_search() {
        assert_eq!(
            typed_target("https://example.com/x"),
            Some("https://example.com/x".to_string())
        );
        assert_eq!(
            typed_target("  example.com  "),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            typed_target("localhost:8392/board"),
            Some("https://localhost:8392/board".to_string())
        );
        assert_eq!(
            typed_target("/tmp/page.html"),
            Some("/tmp/page.html".to_string())
        );
        assert_eq!(
            typed_target("明日の天気"),
            Some(format!(
                "{SEARCH}%E6%98%8E%E6%97%A5%E3%81%AE%E5%A4%A9%E6%B0%97"
            ))
        );
        // 空白が混じれば行き先ではなく探し物。
        assert_eq!(
            typed_target("rust iced webview"),
            Some(format!("{SEARCH}rust+iced+webview"))
        );
        // 数字だけの札は開きに行かない。
        assert_eq!(typed_target("1.2.3"), Some(format!("{SEARCH}1.2.3")));
        assert_eq!(typed_target("   "), None);
        // 読めない綴りは wry へ渡さず探し物に(渡すと窓ごと落ちる)。
        assert_eq!(
            typed_target("http://exa mple.com"),
            Some(format!("{SEARCH}http%3A%2F%2Fexa+mple.com"))
        );
    }

    #[test]
    fn addresses_wry_cannot_read_are_refused_before_they_reach_it() {
        assert!(loadable("https://example.com/x?q=1#top"));
        assert!(loadable("file:///tmp/a%20b.md"));
        for broken in ["", "http://exa mple.com", "http://a b", "https://x.com/\n"] {
            assert!(!loadable(broken), "{broken:?}");
        }
    }

    #[test]
    fn the_typed_address_travels_through_the_new_tab_ipc_port() {
        assert_eq!(
            parse(3, "navigate:9:example.com"),
            Some(Request::OpenBookmark {
                id: 3,
                nonce: 9,
                url: "https://example.com".into(),
            })
        );
        assert_eq!(
            parse(3, "navigate:9:焼き鳥 レシピ"),
            Some(Request::OpenBookmark {
                id: 3,
                nonce: 9,
                url: format!("{SEARCH}%E7%84%BC%E3%81%8D%E9%B3%A5+%E3%83%AC%E3%82%B7%E3%83%94"),
            })
        );
        assert_eq!(parse(3, "navigate:9:   "), None);
        assert_eq!(parse(3, "navigate:nine:example.com"), None);
    }

    #[test]
    fn the_input_moves_through_history_hits_with_control_n_and_p() {
        let html = new_tab_page(&[], &["https://example.com/one".to_string()], 5);
        assert!(html.contains("var SUGGEST=[\"https://example.com/one\"]"));
        assert!(html.contains("lower==='n'||lower==='p'"));
        assert!(html.contains("e.ctrlKey&&!e.metaKey&&!e.altKey"));
        assert!(html.contains("navigate:5:"));
        // 変換確定の Return を渡さない。
        assert!(html.contains("e.isComposing||e.keyCode===229"));
        // 候補は欄の下へ浮かせる。流れに置くと打ち始めた瞬間に欄がずれる。
        assert!(html.contains("#hits{position:absolute"));
        // 焦点が欄に在る回だけ動く。
        assert!(html.contains("input.addEventListener('keydown'"));
        assert!(html.contains("input.addEventListener('blur'"));
    }

    #[test]
    fn the_visit_ledger_keeps_one_row_per_address_newest_first() {
        let list = history::add(Vec::new(), "https://a.example/", 100);
        let list = history::add(list, "https://b.example/", 200);
        let list = history::add(list, "https://a.example/", 300);
        assert_eq!(list.len(), 2, "{list:?}");
        assert_eq!(list[0].spec, "https://a.example/");
        assert_eq!(list[0].at, 300, "時刻が新しいほうへ移っていない");
        assert!(history::add(Vec::new(), "   ", 0).is_empty());

        let mut long = Vec::new();
        for index in 0..(history::MAX + 20) {
            long = history::add(long, &format!("https://例.test/{index}"), index as i64);
        }
        assert_eq!(long.len(), history::MAX);
        assert!(!long.iter().any(|visit| visit.spec.ends_with("/0")));
    }

    #[test]
    fn a_broken_ledger_reads_as_an_empty_shelf_instead_of_killing_the_window() {
        assert!(history::parse("これはJSONではない").is_empty());
        assert!(history::parse("{}").is_empty());
        let got = history::parse(r#"[{"spec":"https://a/","at":1},{"at":2},{"spec":"  "}]"#);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].spec, "https://a/");
    }

    #[test]
    fn bookmarks_ride_behind_the_visits_and_never_show_up_twice() {
        let visits = history::add(
            history::add(Vec::new(), "https://a.example/", 1),
            "https://b.example/",
            2,
        );
        let marks = vec![
            sample_bookmark("栞だけ", "https://c.example/"),
            sample_bookmark("両方に在る", "https://a.example/"),
        ];
        assert_eq!(
            history::merge(&visits, &marks),
            vec![
                "https://b.example/",
                "https://a.example/",
                "https://c.example/"
            ]
        );
    }

    #[test]
    fn command_digits_open_bookmarks_only_while_the_new_tab_is_showing() {
        let marks = vec![
            sample_bookmark("一枚目", "https://one.example/"),
            sample_bookmark("二枚目", "https://two.example/"),
        ];
        assert_eq!(
            bookmark_shortcut(1, 4, 9, &marks),
            Some(Request::OpenBookmark {
                id: 4,
                nonce: 9,
                url: "https://two.example/".into(),
            })
        );
        // 栞が足りない番号は何も起こさない(タブ選択へは落とさない)。
        assert_eq!(bookmark_shortcut(8, 4, 9, &marks), None);
        // 新規タブを見ていない回は従来どおりタブの直接選択。
        assert_eq!(
            contextual_request(Request::SelectTab(2)),
            Some(Request::SelectTab(2)),
            "新規タブが無いのに栞へ振っている"
        );
        assert_eq!(
            contextual_request(Request::CloseTab),
            Some(Request::CloseTab)
        );
    }

    #[test]
    fn removing_a_bookmark_preserves_every_other_json_entry() {
        let mut items = vec![
            serde_json::json!({"title":"one","spec":"https://one.example"}),
            serde_json::json!({"title":"two","spec":"https://two.example"}),
            serde_json::json!({"other":"untouched"}),
        ];
        assert!(remove_bookmark_value(&mut items, "https://one.example"));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["spec"], "https://two.example");
        assert_eq!(items[1]["other"], "untouched");
        assert!(!remove_bookmark_value(
            &mut items,
            "https://missing.example"
        ));
    }

    #[test]
    fn saved_bookmarks_get_a_hover_delete_button() {
        let html_contract = new_tab_html(&[]);
        assert!(html_contract.contains("[data-remove-bookmark]"));
        assert!(html_contract.contains("bookmark:delete:"));
        assert!(html_contract.contains("li:hover .remove"));
    }

    #[test]
    fn browser_history_keys_stop_the_page_from_stealing_navigation() {
        assert!(
            KEYS.contains(
                "history.back(); event.preventDefault(); event.stopImmediatePropagation()"
            )
        );
        assert!(KEYS.contains(
            "history.forward(); event.preventDefault(); event.stopImmediatePropagation()"
        ));
    }

    #[test]
    fn a_default_domain_covers_its_subdomains_but_not_a_lookalike() {
        let sites = merged_translation_sites();
        assert!(shows_original(&sites, "github.com"));
        assert!(shows_original(&sites, "sub.github.com"));
        assert!(shows_original(&sites, "GitHub.com"));
        assert!(!shows_original(&sites, "github.com.evil.jp"));
        assert!(!shows_original(&sites, "evilgithub.com"));
        assert!(!shows_original(&sites, "example.com"));
    }

    #[test]
    fn a_runtime_choice_overrides_the_built_in_default() {
        let mut sites = BTreeMap::new();
        sites.insert("github.com".to_string(), true);
        sites.insert("gist.github.com".to_string(), false);
        // 細かい方が勝つので、既定を原文に据えたまま一部だけ翻訳へ戻せる。
        assert!(shows_original(&sites, "github.com"));
        assert!(shows_original(&sites, "docs.github.com"));
        assert!(!shows_original(&sites, "gist.github.com"));

        sites.insert("github.com".to_string(), false);
        assert!(!shows_original(&sites, "github.com"));
    }

    #[test]
    fn only_a_well_formed_site_choice_reaches_the_store() {
        assert_eq!(
            translation_site_message("trsite:src:GitHub.com."),
            Some(SiteMessage::Set {
                host: "github.com".to_string(),
                original: true,
            })
        );
        assert_eq!(
            translation_site_message("trsite:ja:news.ycombinator.com"),
            Some(SiteMessage::Set {
                host: "news.ycombinator.com".to_string(),
                original: false,
            })
        );
        assert_eq!(
            translation_site_message("trsite:ask:example.com"),
            Some(SiteMessage::Ask("example.com".to_string()))
        );
        assert_eq!(translation_site_message("trsite:src:"), None);
        assert_eq!(translation_site_message("trsite:src:a b.com"), None);
        assert_eq!(translation_site_message("trsite:drop:example.com"), None);
        assert_eq!(translation_site_message("hide"), None);
    }

    #[test]
    fn a_saved_choice_survives_a_round_trip_through_the_tracked_file() {
        let mut sites = BTreeMap::new();
        sites.insert("github.com".to_string(), true);
        sites.insert("docs.python.org".to_string(), false);
        let path = std::env::temp_dir().join(format!(
            "armature-translation-{}.json",
            std::process::id()
        ));
        write_translation_sites(&path, &sites).expect("選択を書ける");
        let source = std::fs::read_to_string(&path).expect("選択を読める");
        let _ = std::fs::remove_file(&path);
        assert_eq!(parse_translation_sites(&source), Some(sites));
    }

    #[test]
    fn the_shipped_defaults_and_the_asset_agree_on_their_shape() {
        assert!(DEFAULT_ORIGINAL_HOSTS.contains(&"github.com"));
        assert!(parse_translation_sites(EMBEDDED_TRANSLATION_SITES).is_some());
        let script = translation_sites_script();
        assert!(script.starts_with(";window.__cockpitTrSites = {"));
        assert!(script.contains("\"github.com\":true"));
    }

    #[test]
    fn the_page_side_mirrors_the_domain_rule_and_asks_before_translating() {
        // JS の original() は shows_original と同じ「完全一致か`.`後方一致」で測る。
        assert!(TRANSLATION.contains("name.slice(-(entry.length + 1)) !== '.' + entry"));
        assert!(TRANSLATION.contains("if (entry.length > best)"));
        assert!(TRANSLATION.contains("say('trsite:ask:' + host())"));
        assert!(TRANSLATION.contains("say('trsite:' + (next ? 'src' : 'ja') + ':' + host())"));
        // 入力中の t は切替に使わない——選択がそのまま保存されてしまうため。
        assert!(
            TRANSLATION.contains(
                "if (event.metaKey || event.ctrlKey || event.altKey || typing()) return;"
            )
        );
        // 原文を選んでいる間は自動翻訳を仕掛けない。
        assert!(TRANSLATION.contains("function scheduleAuto(delay) {\n    if (paused) return;"));
    }

    #[test]
    fn translation_wait_has_a_bounded_failure_path() {
        let _ja = armature_core::lang::scoped(armature_core::lang::Lang::Ja);
        let script = translation_script();
        assert!(script.contains("翻訳の返事が届かなかった"));
        assert!(script.contains("翻訳を頼めなかった"));
        assert!(script.contains("var TARGET = 'ja';"));
        assert!(!script.contains("__ARMATURE_"));
        assert!(TRANSLATION.contains("clearTimeout(timer)"));
    }

    #[test]
    fn japanese_batch_does_not_stop_later_translation_batches() {
        let _ja = armature_core::lang::scoped(armature_core::lang::Lang::Ja);
        let original = vec!["日本語とAPI".to_string()];
        let got = automatic_translation_reply(
            &original,
            Ok(armature_core::page_trans::Translated {
                texts: vec!["書き換えられた文".to_string()],
                lang: "ja".to_string(),
            }),
        );
        assert_eq!(got, (true, original, String::new()));
    }

    #[test]
    fn a_batch_already_in_english_is_left_alone_when_translating_into_english() {
        let _en = armature_core::lang::scoped(armature_core::lang::Lang::En);
        let original = vec!["Plain English".to_string()];
        let got = automatic_translation_reply(
            &original,
            Ok(armature_core::page_trans::Translated {
                texts: vec!["rewritten".to_string()],
                lang: "en".to_string(),
            }),
        );
        assert_eq!(got, (true, original, String::new()));
    }

    #[test]
    fn local_markdown_is_rendered_as_utf8_html_inside_the_browser() {
        let path =
            std::env::temp_dir().join(format!("armature-markdown-{}.md", std::process::id()));
        std::fs::write(&path, "# 日本語\n\n本文\n").expect("試験用Markdownを書ける");
        let source = source_for_path(&path);
        let _ = std::fs::remove_file(path);
        let Source::HtmlFile { title, html, url } = source else {
            panic!("MarkdownはHTMLとして開く");
        };
        assert!(title.ends_with(".md"));
        assert!(url.starts_with("file:///"));
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<h1>日本語</h1>"));
        assert!(html.contains("本文"));
        assert!(!html.contains('\u{fffd}'));
    }

    #[test]
    fn saved_markdown_url_is_rendered_again_after_restart() {
        let path = std::env::temp_dir().join(format!(
            "cockpit iced #saved-markdown-{}.md",
            std::process::id()
        ));
        std::fs::write(&path, "# 再起動後\n").expect("試験用Markdownを書ける");
        let url = file_url(&path);
        let source = source_for_saved_url(&url);
        let _ = std::fs::remove_file(path);
        let Source::HtmlFile {
            title,
            html,
            url: restored_url,
        } = source
        else {
            panic!("保存済みMarkdownもHTMLとして復元する");
        };
        assert!(title.ends_with(".md"));
        assert_eq!(restored_url, url);
        assert!(html.contains("<h1>再起動後</h1>"));
    }

    #[test]
    fn a_markdown_url_opens_as_html_even_when_percent_encoded() {
        // 頁の外から来た md は素のまま WebView へ渡すと字が化けた(2026-09-21)。
        let dir = std::env::temp_dir().join(format!("armature-md-url-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("試験の置き場を作れる");
        let path = dir.join("検収 表.md");
        std::fs::write(&path, "| 列 | 値 |\n|---|---|\n| 甲 | 1 |\n").expect("試験用Markdownを書ける");
        let raw = format!("file://{}", path.display());
        let encoded = format!(
            "file://{}",
            path.display()
                .to_string()
                .replace("検収", "%E6%A4%9C%E5%8F%8E")
                .replace(' ', "%20")
        );
        let from_raw = source_for_url(&raw);
        let from_encoded = source_for_url(&encoded);
        let html_url = format!("file://{}", dir.join("a.html").display());
        let html = source_for_url(&html_url);
        let _ = std::fs::remove_dir_all(&dir);
        for source in [&from_raw, &from_encoded] {
            let Source::HtmlFile { html, .. } = source else {
                panic!("外から来た md も組み立てた HTML で開く");
            };
            assert!(html.contains("<meta charset=\"utf-8\">"));
            assert!(html.contains("<table>"));
        }
        assert_eq!(from_raw.identity(), from_encoded.identity(), "綴りが違っても同じタブ");
        assert!(
            matches!(&html, Source::Url(url) if *url == html_url),
            "HTML は届いた綴りのまま"
        );
    }

    #[test]
    fn external_open_hands_markdown_over_as_html_outside_the_vault() {
        let base = std::env::temp_dir().join(format!("armature-external-{}", std::process::id()));
        let vault = base.join("vault");
        let out = base.join("out");
        std::fs::create_dir_all(&vault).expect("試験の置き場を作れる");
        let md = vault.join("メモ.md");
        std::fs::write(&md, "# 見出し\n").expect("試験用Markdownを書ける");
        let target = external_target(&file_url(&md), &out).expect("md は外へ渡せる");
        let written = std::fs::read_to_string(&target).unwrap_or_default();
        let beside = std::fs::read_dir(&vault).map(Iterator::count).unwrap_or(0);
        let _ = std::fs::remove_dir_all(&base);
        assert!(target.starts_with(&out.to_string_lossy().into_owned()));
        assert!(target.ends_with("メモ.md.html"));
        assert!(written.contains("<meta charset=\"utf-8\">"));
        assert!(written.contains("<h1>見出し</h1>"));
        assert_eq!(beside, 1, "md の隣に .html を撒かない");
        assert_eq!(
            external_target("https://example.com/a", &out),
            Ok("https://example.com/a".to_string())
        );
        assert!(external_target("cockpit-new-tab:3", &out).is_err());
    }

    #[test]
    fn config_and_unknown_utf8_files_open_as_bounded_text_pages() {
        for extension in ["conf", "unknown"] {
            let path = std::env::temp_dir().join(format!(
                "armature-text-{}.{}",
                std::process::id(),
                extension
            ));
            std::fs::write(&path, "名前 = \"設定\"\n<unsafe>\n").expect("試験用テキストを書ける");
            let source = source_for_path(&path);
            let _ = std::fs::remove_file(path);
            let Source::HtmlFile { html, .. } = source else {
                panic!("UTF-8テキストはWebKitへ生ファイルで渡さない");
            };
            assert!(html.contains("名前 = &quot;設定&quot;"));
            assert!(html.contains("&lt;unsafe&gt;"));
            assert!(!html.contains("<unsafe>"));
        }
    }

    #[test]
    fn explorer_reads_only_the_bounded_prefix_of_large_text_files() {
        let path = std::env::temp_dir().join(format!(
            "armature-bounded-text-{}.conf",
            std::process::id()
        ));
        std::fs::write(&path, b"abcdefghij").expect("試験用テキストを書ける");
        let bytes = read_prefix(&path, 4).expect("先頭だけ読める");
        let _ = std::fs::remove_file(path);
        assert_eq!(bytes, b"abcd");
    }

    #[test]
    fn webkit_native_formats_are_not_loaded_into_rust_memory() {
        assert!(browser_native_file("pdf"));
        assert!(browser_native_file("PNG"));
        assert!(browser_native_file("mp4"));
        assert!(!browser_native_file("conf"));
    }

    #[test]
    fn html_backed_markdown_keeps_its_file_identity_on_about_blank() {
        let current = "file:///tmp/%E6%97%A5%E6%9C%AC%E8%AA%9E.md";
        assert_eq!(accepted_location(current, "about:blank"), current);
        assert_eq!(
            accepted_location(current, "https://example.com/next"),
            "https://example.com/next"
        );
    }

    #[test]
    fn every_new_tab_gets_a_distinct_internal_identity() {
        let first = new_tab_source(&[]).identity();
        let second = new_tab_source(&[]).identity();
        assert_ne!(first, second);
    }

    #[test]
    fn local_bookmark_paths_become_escaped_file_urls() {
        let path = std::env::temp_dir().join(format!(
            "cockpit iced #bookmark-{}.html",
            std::process::id()
        ));
        std::fs::write(&path, "<html></html>").expect("write fixture");
        let got = bookmark_target(&path.to_string_lossy()).expect("absolute file path");
        assert!(got.starts_with("file://"));
        assert!(got.contains("%20"));
        assert!(got.contains("%23"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn hint_new_tab_messages_are_parsed_with_their_full_url() {
        assert_eq!(
            parse(3, "tab:open:https://example.com/a?q=1:2"),
            Some(Request::NewTab("https://example.com/a?q=1:2".into()))
        );
    }

    #[test]
    fn bookmark_messages_keep_the_new_tab_nonce_and_payload() {
        assert_eq!(
            parse(7, "bookmark:open:42:https://example.com/a?q=1:2"),
            Some(Request::OpenBookmark {
                id: 7,
                nonce: 42,
                url: "https://example.com/a?q=1:2".into(),
            })
        );
        assert_eq!(
            parse(9, "bookmark:delete:18:file:///tmp/a:b.html"),
            Some(Request::DeleteBookmark {
                id: 9,
                nonce: 18,
                spec: "file:///tmp/a:b.html".into(),
            })
        );
    }

    #[test]
    fn saved_browser_tabs_return_without_entering_browser_mode() {
        let snapshot =
            parse_saved("First\thttps://example.com/one\t0\nSecond\thttps://example.com/two\t1\n");
        assert_eq!(snapshot.tabs.len(), 2);
        assert_eq!(snapshot.active, 1);
        assert!(!snapshot.visible);
    }

    #[test]
    fn bounds_are_passed_to_wry_without_scaling_twice() {
        let bounds = Bounds {
            x: 12.0,
            y: 34.0,
            width: 560.0,
            height: 780.0,
        };
        let got = rect(bounds);
        assert_eq!(
            got.position.to_logical::<f64>(1.0),
            LogicalPosition::new(12.0, 34.0)
        );
        assert_eq!(
            got.size.to_logical::<f64>(1.0),
            LogicalSize::new(560.0, 780.0)
        );
    }

    #[test]
    fn 窓が育ったのに古い枠のままなら置き直す() {
        // 窓が生まれた頃(1440×900)の枠と、全画面(1920×1080)の枠。
        // 復元の便と全画面復元が競って前者が残るのが今回の症状で、
        // 頁はパネルの左上に小さく寄って出た。
        let born = Bounds { x: 359.0, y: 68.0, width: 709.0, height: 816.0 };
        let grown = Bounds { x: 484.0, y: 68.0, width: 952.0, height: 996.0 };
        assert!(drifted(born, grown));
        // 同じ枠・丸めの端数だけの違いでは動かさない。
        assert!(!drifted(grown, grown));
        assert!(!drifted(
            grown,
            Bounds { x: 484.4, y: 67.7, ..grown }
        ));
    }

    #[test]
    fn パネルに収まる版面まで拡大率を絞る() {
        // 実測のパネル(2560×1440の机・文字倍率1.4)。ここで 1.4 を素通しすると
        // 版面が 920px 台へ落ち、机向けに組まれた頁が横にはみ出す。
        let width = 1293.7;
        let zoom = fit_zoom(1.4, width);
        assert!(zoom < 1.4, "パネルの幅で頭を押さえていない: {zoom}");
        // 押さえた結果の版面が FIT_WIDTH ちょうどに乗る。
        assert!((width / zoom - FIT_WIDTH).abs() < 1.0, "版面がずれる: {zoom}");
    }

    #[test]
    fn パネルが広ければ利用者の選んだ倍率をそのまま使う() {
        // 天井は利用者の倍率。**パネルが広いからといって勝手に拡大しない。**
        assert!((fit_zoom(1.4, 3000.0) - 1.4).abs() < 1e-6);
        assert!((fit_zoom(1.0, 3000.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn パネルが細っても読める下限で止まる() {
        assert!((fit_zoom(1.4, 200.0) - MIN_FIT_ZOOM).abs() < 1e-6);
        // パネルの幅を知らないうちは天井のまま。
        assert!((fit_zoom(1.4, 0.0) - 1.4).abs() < 1e-6);
    }

    #[test]
    fn 横のオーバースクロールは殺さない() {
        // 二本指の戻る/進むは overscroll-behavior-x が none だと消える。
        assert!(PAGE_SURFACE.contains("overscroll-behavior-y:none"));
        assert!(!PAGE_SURFACE.contains("overscroll-behavior:none"));
    }

    #[test]
    fn the_page_zoom_follows_the_window_font_scale() {
        // 窓の倍率と同じ数字で揃える(別々の刻みだと押すたびに離れていく)。
        assert!((page_zoom(1.0) - 1.0).abs() < f64::EPSILON);
        assert!((page_zoom(crate::font::MAX_SCALE) - 1.8).abs() < 1e-6);
        assert!((page_zoom(crate::font::MIN_SCALE) - 0.7).abs() < 1e-6);
        // 壊れた値で WebKit に極端な拡大を渡さない。
        assert!((page_zoom(f32::NAN) - 1.0).abs() < f64::EPSILON);
        assert!((page_zoom(999.0) - 5.0).abs() < f64::EPSILON);
    }
}
