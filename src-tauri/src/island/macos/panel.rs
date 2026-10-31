//! 岛的窗口换成不激活的 NSPanel：点了不激活 Hindsight，不接键盘；放在菜单栏上面一层，每个桌面都有。

use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, ClassType, MainThreadOnly};
use objc2_app_kit::{
    NSColor, NSPanel, NSResponder, NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol};

define_class!(
    // 岛的窗口建好以后换成这个类。只有 NSPanel 能做到点了不激活 app
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "HindsightIslandPanel"]
    struct Panel;

    impl Panel {
        // 岛不接键盘：成了 key window，用户正在打字的 app 就收不到键盘了
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            false
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }
    }
);

/// 把 Tauri 建的窗口换成 [`Panel`]：不激活、透明、没有阴影，在菜单栏上面一层。
pub(super) fn set_up(ns_window: &Retained<NSWindow>) {
    // SAFETY: Panel 没有自己的实例变量，对象只会比它要的大。原来的类是 tao 的 NSWindow 子类，
    // 它改写的方法 Panel 都改写了或者用不到；它的实例变量只有 `set_focusable` 会读，岛不调用
    unsafe {
        objc2::ffi::object_setClass(
            Retained::as_ptr(ns_window).cast_mut().cast(),
            Panel::class(),
        );
    }
    ns_window.setStyleMask(ns_window.styleMask() | NSWindowStyleMask::NonactivatingPanel);
    // 窗口建好以后才改样式，窗口服务器那边不跟着变，点了还是激活 app（实测）。AppKit 自己用这个
    // 私有方法同步；没有这个方法就不调：点岛会激活 Hindsight，别的照常
    let prevents = sel!(_setPreventsActivation:);
    if ns_window.respondsToSelector(prevents) {
        // SAFETY: 上面查过有这个方法，它收一个 BOOL
        let _: () = unsafe { msg_send![&**ns_window, _setPreventsActivation: true] };
    }
    // NSPanel 默认在 app 不是前台时藏起来，岛要一直在
    ns_window.setHidesOnDeactivate(false);
    ns_window.setOpaque(false);
    ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
    ns_window.setHasShadow(false);
    // Above the menu bar
    ns_window.setLevel(NSStatusWindowLevel + 1);
}

/// Returns how the island's window shows across desktops: on every desktop, still while the
/// desktops switch, and skipped when the user switches windows.
///
/// The island shows over full-screen apps only on a screen with a notch. There, a full-screen app
/// draws below the notch by default and leaves a black band at the top, where the closed island
/// sits. Without a notch, full-screen content reaches the top of the screen, and the island would
/// cover it.
pub(super) fn collection_behavior(notch: bool) -> NSWindowCollectionBehavior {
    let always = NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::Stationary
        | NSWindowCollectionBehavior::IgnoresCycle;
    if notch {
        always | NSWindowCollectionBehavior::FullScreenAuxiliary
    } else {
        always
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 有刘海时，全屏 app 的内容在刘海下面，岛落在顶上的黑边里；没有刘海时，岛会盖住全屏的内容
    #[test]
    fn the_island_shows_over_full_screen_apps_only_beside_a_notch() {
        let full_screen = NSWindowCollectionBehavior::FullScreenAuxiliary;
        assert!(collection_behavior(true).contains(full_screen));
        assert!(!collection_behavior(false).contains(full_screen));
        // 其余和原来一样：每个桌面都有，切桌面时不跟着动，Cmd+` 切窗口时跳过它
        let always = NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle;
        for notch in [true, false] {
            assert!(collection_behavior(notch).contains(always));
        }
    }
}
