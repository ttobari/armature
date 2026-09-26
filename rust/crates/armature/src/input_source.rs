//! キーボード入力ソースを英字へ倒す。
//!
//! 常駐ヘルパーも別プロセスも挟まない——このアプリが自分で Carbon の
//! Text Input Services を叩く。右パネルの操作は j/k の1字キーなので、
//! かな入力のままだと変換に食われて動かない。
//!
//! 使うのは `TISCopyCurrentASCIICapableKeyboardInputSource`。いま選ばれている
//! 入力ソースに対応する「ASCII が打てる版」を返してくれるので、こちらで
//! 入力ソースIDを名指しする必要がない。US配列でもDvorakでも、利用者が普段
//! 使っている英字配列がそのまま選ばれる。
//!
//! **AppKit の `NSTextInputContext` へ寄せてはいけない**(2026-08-22 に一度寄せて
//! 差し戻した)。あちらは `setAllowedInputSourceLocales` で「英字だけを許す」枷を
//! 掛ける方式で、掛かっている間は利用者が手で「かな」を押しても日本語へ戻せない。
//! 解除は `restore` を通る経路だけなので、そこを通らずに抜けると**日本語が一切
//! 打てない窓**が残る。TIS 版は
//! 「切り替える」だけなので、失敗しても手で戻せるという逃げ道がある。
//! macOS 26 の legacy 警告を避ける目的だったが、winit 0.30 自身が Carbon を
//! リンクしているため、こちらをやめても警告は消えない——避ける利がない。

use std::ffi::c_void;
use std::sync::Mutex;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentASCIICapableKeyboardInputSource() -> *mut c_void;
    fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
    fn TISSelectInputSource(source: *mut c_void) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *mut c_void);
}

/// 倒す前に選ばれていた入力ソース。中央へ戻るときにここへ返す。
///
/// TIS のオブジェクトをそのまま控える(Copy 系が返す参照は解放するまで
/// 生きている)。生ポインタは `Send` でないので数として持つ。
static PREVIOUS: Mutex<Option<usize>> = Mutex::new(None);

/// 英字入力へ倒す。切り替えられたら `true`。
///
/// すでに英字ならそのまま。失敗しても呼び手は困らない(キー操作が
/// 効かないだけで、描画も入力も止まらない)ので結果は捨ててよい。
pub fn to_ascii() -> bool {
    remember_current();
    // SAFETY: Copy 系APIは所有権つきの参照を返す。選び終えたら必ず解放する。
    unsafe {
        let source = TISCopyCurrentASCIICapableKeyboardInputSource();
        if source.is_null() {
            return false;
        }
        let status = TISSelectInputSource(source);
        CFRelease(source);
        status == 0
    }
}

/// 倒す前のものを控える。**既に控えがあるなら触らない**——右パネルの中を
/// 渡り歩くたびに控えを取り直すと、戻す先が英字で上書きされてしまう。
fn remember_current() {
    let Ok(mut previous) = PREVIOUS.lock() else {
        return;
    };
    if previous.is_some() {
        return;
    }
    // SAFETY: 上と同じ。控えている間は解放せず、restore が選び直してから解放する。
    let current = unsafe { TISCopyCurrentKeyboardInputSource() };
    if !current.is_null() {
        *previous = Some(current as usize);
    }
}

/// 控えた入力ソースへ返す。控えが無ければ何もしない。
///
/// 端末は日本語を打つ場所なので、右パネルから戻ったときに英字のままだと
/// 毎回手で切り替え直すことになる。
pub fn restore() -> bool {
    let source = {
        let Ok(mut previous) = PREVIOUS.lock() else {
            return false;
        };
        let Some(source) = previous.take() else {
            return false;
        };
        source
    };
    // SAFETY: remember_current が控えた参照。選び直したら手放す。
    unsafe {
        let source = source as *mut c_void;
        let status = TISSelectInputSource(source);
        CFRelease(source);
        status == 0
    }
}
