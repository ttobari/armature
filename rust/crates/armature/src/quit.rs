//! ⌘Q と Dock の「終了」を、窓の閉じる釦と同じ道へ寄せる。
//!
//! winit の既定のメニューの「終了」は `terminate:` で、AppKit はそのままプロセスを
//! 終える——iced の update を通らないので、外から足したパネルの `Panel::on_exit` が
//! 走らない。閉じる釦は `CloseRequested` として update へ届き、`Cockpit::exit_now` が
//! パネルに知らせてから終える。そこで終了の入口2つ(メニューの ⌘Q と、Dock・ログアウト・
//! AppleScript の quit の Apple Event)を、主窓の `performClose:` に掛け替える。

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSMenu, NSWindow};

/// Apple Event の組。`kCoreEventClass`('aevt')の `kAEQuitApplication`('quit')。
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const QUIT_APPLICATION: u32 = u32::from_be_bytes(*b"quit");

pub(crate) struct Ivars {
    window: Retained<NSWindow>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    pub(crate) struct Quitter;

    unsafe impl NSObjectProtocol for Quitter {}

    impl Quitter {
        /// メニューの ⌘Q。
        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            self.close();
        }

        /// Dock の「終了」・ログアウト・AppleScript の quit。
        #[unsafe(method(handleQuit:withReplyEvent:))]
        fn handle_quit(&self, _event: &AnyObject, _reply: &AnyObject) {
            self.close();
        }
    }
);

impl Quitter {
    fn close(&self) {
        self.ivars().window.performClose(None);
    }
}

/// 終了の入口を `window` の閉じる釦へ寄せる。窓が開いたあと、窓の糸で1度だけ呼ぶ。
pub(crate) fn route_to_close(window: Retained<NSWindow>, mtm: MainThreadMarker) {
    let quitter = mtm.alloc::<Quitter>().set_ivars(Ivars { window });
    // SAFETY: NSObject の init。
    let quitter: Retained<Quitter> = unsafe { msg_send![super(quitter), init] };
    if let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() {
        retarget(&menu, &quitter);
    }
    // SAFETY: 受け手は `handleQuit:withReplyEvent:` を持つ。組の2つは FourCharCode(u32)。
    unsafe {
        let manager: Option<Retained<AnyObject>> =
            msg_send![objc2::class!(NSAppleEventManager), sharedAppleEventManager];
        if let Some(manager) = manager {
            let _: () = msg_send![
                &*manager,
                setEventHandler: &*quitter,
                andSelector: sel!(handleQuit:withReplyEvent:),
                forEventClass: CORE_EVENT_CLASS,
                andEventID: QUIT_APPLICATION
            ];
        }
    }
    // メニューの target も Apple Event の受け手も相手を保持しない。窓と同じだけ生かす。
    std::mem::forget(quitter);
}

/// `terminate:` を持つ項目(「Armature を終了」)を、`quitter` の `quit:` へ付け替える。
fn retarget(menu: &NSMenu, quitter: &Quitter) {
    let target: &AnyObject = quitter.as_ref();
    for item in menu.itemArray().iter() {
        if item.action() == Some(sel!(terminate:)) {
            // SAFETY: target は `quit:` を持ち、アプリが終わるまで生きている。
            unsafe {
                item.setTarget(Some(target));
                item.setAction(Some(sel!(quit:)));
            }
        }
        if let Some(submenu) = item.submenu() {
            retarget(&submenu, quitter);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quit_event_is_aevt_quit() {
        assert_eq!(CORE_EVENT_CLASS, 0x6165_7674);
        assert_eq!(QUIT_APPLICATION, 0x7175_6974);
    }
}
