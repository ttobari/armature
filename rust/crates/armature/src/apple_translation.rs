//! Apple Translation.framework をアプリ束へ同梱する小さな橋。
//!
//! 常駐ヘルパーや別プロセスは作らない。Swift限定APIとの境界だけをC ABIにし、
//! 訳文は呼び出し元のRustスレッドへ一度だけ返す。
//!
//! 橋(`libCockpitTranslation.dylib`)は**起動後に自分で読み込む**(`dlopen`)。リンク時に
//! 結ぶと、Armature を依存に取った別の crate(自分のパネルを足した版)の実体が rpath を
//! 持たず、起動した瞬間に落ちる。読めなければ翻訳と画像の貼り付けだけが黙って止まる。

use armature_core::tr;
use std::ffi::{CStr, CString, c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::sync::{OnceLock, mpsc};
use std::time::Duration;

use armature_core::page_trans::Translated;

type Reply = Result<String, String>;

type Callback = extern "C" fn(*mut c_void, *const c_char);

/// 橋が出している関数。
struct Bridge {
    translate: unsafe extern "C" fn(*const c_char, *const c_char, *mut c_void, Callback),
    has_image: unsafe extern "C" fn() -> bool,
    text: unsafe extern "C" fn() -> *mut c_char,
    free: unsafe extern "C" fn(*mut c_char),
}

/// 橋を1度だけ読む。束の `Contents/Frameworks` と、実体の隣(`cargo run` のとき)を探す。
fn bridge() -> Option<&'static Bridge> {
    static BRIDGE: OnceLock<Option<Bridge>> = OnceLock::new();
    BRIDGE
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let dir = exe.parent()?;
            [
                dir.join("../Frameworks/libCockpitTranslation.dylib"),
                dir.join("libCockpitTranslation.dylib"),
            ]
            .iter()
            .find_map(|path| open_bridge(path))
        })
        .as_ref()
}

fn open_bridge(path: &std::path::Path) -> Option<Bridge> {
    let name = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: NUL 終端のパス。読めなければ null が返るだけ。
    let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return None;
    }
    let symbol = |name: &CStr| {
        // SAFETY: 開けた handle から名前で引くだけ。無ければ null。
        let found = unsafe { libc::dlsym(handle, name.as_ptr()) };
        (!found.is_null()).then_some(found)
    };
    // SAFETY: どれも Swift 側の `@_cdecl` で、宣言した型のまま出している関数。
    unsafe {
        Some(Bridge {
            translate: std::mem::transmute::<*mut c_void, _>(symbol(c"cockpit_apple_translate")?),
            has_image: std::mem::transmute::<*mut c_void, _>(symbol(c"cockpit_clipboard_has_image")?),
            text: std::mem::transmute::<*mut c_void, _>(symbol(c"cockpit_clipboard_text")?),
            free: std::mem::transmute::<*mut c_void, _>(symbol(c"cockpit_free_string")?),
        })
    }
}

extern "C" fn receive(context: *mut c_void, output: *const c_char) {
    if context.is_null() {
        return;
    }
    // SAFETY: translate が Box::into_raw したSenderをSwift側が一度だけ返す。
    let sender = unsafe { Box::from_raw(context.cast::<mpsc::Sender<Reply>>()) };
    let result = if output.is_null() {
        Err(tr!("Apple Translation returned nothing", "Apple翻訳から空の応答が来た").to_string())
    } else {
        // SAFETY: callback中だけ有効なNUL終端UTF-8文字列。
        unsafe { CStr::from_ptr(output) }
            .to_str()
            .map(str::to_owned)
            .map_err(|error| format!("Apple Translation reply is not UTF-8: {error}"))
    };
    let _ = sender.send(result);
}

/// クリップボードが画像を持っているか。
///
/// 貼る仕事はしない——Claude Code は Ctrl+V を受けると自分でクリップボードから
/// 画像を取り込むので、こちらはその合図を送ってよいかだけを見る。
pub fn clipboard_has_image() -> bool {
    // SAFETY: 引数も戻りの所有権も無い、真偽値を返すだけの呼び出し。
    bridge().is_some_and(|bridge| unsafe { (bridge.has_image)() })
}

/// クリップボードの字を**その場で**読む。
///
/// iced の `clipboard::read` は非同期で、答えが返るのは次の周回。音声入力
/// (音声入力のアプリ)は ⌘V を送った直後に元のクリップボードへ戻すので、周回を1つ
/// 待つと戻されたあとの中身を読んでしまう。
/// ここは ⌘V を受けた同じ呼び出しの中で読み切る。
pub fn clipboard_text() -> Option<String> {
    // SAFETY: Swift 側が strdup した NUL 終端 UTF-8 か null を返す。
    // 読み終えたら同じ橋の `cockpit_free_string` で必ず返す。
    let bridge = bridge()?;
    unsafe {
        let pointer = (bridge.text)();
        if pointer.is_null() {
            return None;
        }
        let text = CStr::from_ptr(pointer).to_str().map(str::to_owned).ok();
        (bridge.free)(pointer);
        text
    }
}

pub fn translate(texts: &[String]) -> Result<Translated, String> {
    if texts.is_empty() {
        return Err(tr!("Nothing to translate", "訳す本文が無い").to_string());
    }
    let input = CString::new(
        serde_json::to_string(texts).map_err(|error| format!("can't encode the text to translate: {error}"))?,
    )
    .map_err(|_| "the text to translate contains NUL".to_string())?;
    let bridge = bridge().ok_or_else(|| {
        tr!(
            "The translation bridge is missing from the app",
            "翻訳の橋がアプリに無い",
        )
        .to_string()
    })?;
    let (sender, receiver): (mpsc::Sender<Reply>, mpsc::Receiver<Reply>) = mpsc::channel();
    let context = Box::into_raw(Box::new(sender)).cast::<c_void>();
    // SAFETY: Swift橋は入力を呼出中にコピーし、contextをcallbackへ一度だけ返す。
    // 訳す先はいまの表示の言語。
    let target = CString::new(armature_core::lang::current().key()).unwrap_or_default();
    unsafe { (bridge.translate)(input.as_ptr(), target.as_ptr(), context, receive) };
    let json = receiver
        .recv_timeout(Duration::from_secs(90))
        .map_err(|_| {
            tr!(
                "Apple Translation didn't answer within 90 seconds",
                "Apple翻訳が90秒以内に返らなかった",
            )
            .to_string()
        })??;
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|error| format!("Apple Translation sent broken JSON: {error}"))?;
    if !value
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return Err(value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(tr!("Apple Translation failed", "Apple翻訳に失敗した"))
            .to_string());
    }
    let translated = value
        .get("texts")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "Apple Translation reply has no translations".to_string())?
        .iter()
        .map(|value| value.as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    if translated.len() != texts.len() {
        return Err("Apple Translation returned a different number of lines".to_string());
    }
    Ok(Translated {
        texts: translated,
        lang: value
            .get("lang")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("en")
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_batch_never_enters_the_system_framework() {
        assert!(translate(&[]).is_err());
    }
}
