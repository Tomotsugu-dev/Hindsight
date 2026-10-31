//! The island window on macOS. The window is always the size of the open island at its largest. A
//! Core Animation mask shows only the closed shapes or the open panel, so the window never resizes
//! and the page never re-lays out while it animates. A tracking area tells when the mouse goes over
//! the island and leaves it, also while another app is active. 窗口是不激活的 NSPanel：点岛不会
//! 让 Hindsight 变成前台 app，用户正在用的 app 还在前台。
//!
//! AppKit objects may only be used on the main thread, so the island lives there.
//!
//! When the content changes:
//! 1. this sends it to the page;
//! 2. the page draws it, measures the closed content and the open panel, and calls [`measured`],
//!    which returns where each side goes;
//! 3. the page puts the content there and calls [`ready`]. Only then does the window show, so it
//!    never shows content that is not in place.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    define_class, msg_send, sel, AllocAnyThread, ClassType, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSColor, NSEvent, NSPanel, NSResponder,
    NSScreen, NSStatusWindowLevel, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
};
use objc2_quartz_core::{
    kCAMediaTimingFunctionEaseInEaseOut, kCAMediaTimingFunctionEaseOut, CACornerMask, CALayer,
    CAMediaTimingFunction, CATransaction,
};
use tauri::{AppHandle, WebviewWindow};

use super::content::Content;
use super::geometry::{self, Closed, Notch, Rect, Screen, NOTCH_RADIUS, OPEN_RADIUS};
use super::{
    emit, new_window, Layout, PageRect, PanelSize, State, CONTENT_EVENT, LAYOUT_EVENT, OPEN_EVENT,
};

/// How long opening takes, in seconds
const OPEN_SECONDS: f64 = 0.32;
/// How long closing takes, in seconds
const CLOSE_SECONDS: f64 = 0.24;

struct Island {
    window: WebviewWindow,
    ns_window: Retained<NSWindow>,
    view: Retained<NSView>,
    mask: Retained<CALayer>,
    /// Two black layers in the mask. Open, both fill the panel. Closed, each covers one shape, or
    /// both cover the only shape, so a side can come and go without adding or removing a layer
    shapes: [Retained<CALayer>; 2],
    observer: Retained<Observer>,
    area: Retained<NSTrackingArea>,
    screen: Screen,
    state: State,
}

thread_local! {
    static ISLAND: RefCell<Option<Island>> = const { RefCell::new(None) };
}

define_class!(
    // Gets the mouse going over the island and leaving it, and changes of the screens
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "HindsightIslandObserver"]
    struct Observer;

    impl Observer {
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            with_island(|island| island.set_open(true));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            with_island(|island| island.set_open(false));
        }

        #[unsafe(method(screensChanged:))]
        fn screens_changed(&self, _notification: &NSNotification) {
            let mtm = self.mtm();
            with_island(|island| island.screens_changed(mtm));
        }
    }
);

impl Observer {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

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

/// Shows `content` on the island in place of what is there. Opens the window the first time.
pub fn show(app: &AppHandle, content: Content) {
    let handle = app.clone();
    on_main_thread(app, move |mtm| {
        ISLAND.with(|cell| {
            let Ok(mut island) = cell.try_borrow_mut() else {
                log::warn!("island: already in use");
                return;
            };
            if island.is_none() {
                match Island::new(&handle, mtm) {
                    Ok(new) => *island = Some(new),
                    Err(e) => {
                        log::warn!("island: cannot open the window: {e}");
                        return;
                    }
                }
            }
            if let Some(island) = island.as_mut() {
                island.set_content(Some(content));
            }
        });
    });
}

/// Takes everything off the island and hides the window.
pub fn hide(app: &AppHandle) {
    on_main_thread(app, |_| {
        with_island(|island| island.set_content(None));
    });
}

/// Returns what is on the island now, for the page when it loads. 要在主线程上调用。
pub fn content() -> Option<Content> {
    main_thread()?;
    with_island(|island| island.state.content.clone()).flatten()
}

/// Takes the widths of the closed content and the size of the open content measured by the page,
/// and returns where each side goes. `None` when there is no island. 要在主线程上调用。
pub fn measured(left: Option<f64>, right: Option<f64>, panel: Option<PanelSize>) -> Option<Layout> {
    main_thread()?;
    with_island(|island| {
        island.state.measured(left, right, panel);
        // A side that grows or shrinks in view moves smoothly
        island.draw(island.ns_window.isVisible());
        island.track();
        island.layout()
    })
}

/// Shows the window: the page has put the content in place. Does nothing when nothing is on the
/// island. 要在主线程上调用。
pub fn ready() {
    if main_thread().is_none() {
        return;
    }
    with_island(|island| {
        if island.state.content.is_some() {
            island.ns_window.orderFrontRegardless();
        }
    });
}

impl Island {
    fn new(app: &AppHandle, mtm: MainThreadMarker) -> Result<Self, String> {
        let screen = main_screen(mtm).ok_or("there is no screen")?;
        // A click on the island works while another app is active
        let window = new_window(app, |builder| builder.accept_first_mouse(true))?;
        Self::set_up(window.clone(), screen, mtm).inspect_err(|_| {
            // Or the next try would fail too: the label is taken
            let _ = window.destroy();
        })
    }

    /// Makes `window` an island: above the menu bar, on every desktop, showing only what the mask
    /// covers.
    fn set_up(
        window: WebviewWindow,
        screen: Screen,
        mtm: MainThreadMarker,
    ) -> Result<Self, String> {
        let ns_window = window.ns_window().map_err(|e| e.to_string())?;
        // SAFETY: Tauri gives the window's NSWindow, which lives as long as the window
        let ns_window = unsafe { Retained::retain(ns_window.cast::<NSWindow>()) }
            .ok_or("the window has no NSWindow")?;
        // SAFETY: Panel 没有自己的实例变量，对象只会比它要的大。原来的类是 tao 的 NSWindow 子类，
        // 它改写的方法 Panel 都改写了或者用不到；它的实例变量只有 `set_focusable` 会读，岛不调用
        unsafe {
            objc2::ffi::object_setClass(
                Retained::as_ptr(&ns_window).cast_mut().cast(),
                Panel::class(),
            );
        }
        ns_window.setStyleMask(ns_window.styleMask() | NSWindowStyleMask::NonactivatingPanel);
        // 窗口建好以后才改样式，窗口服务器那边不跟着变，点了还是激活 app（实测）。AppKit 自己用这个
        // 私有方法同步；没有这个方法就不调：点岛会激活 Hindsight，别的照常
        let prevents = sel!(_setPreventsActivation:);
        if ns_window.respondsToSelector(prevents) {
            // SAFETY: 上面查过有这个方法，它收一个 BOOL
            let _: () = unsafe { msg_send![&*ns_window, _setPreventsActivation: true] };
        }
        // NSPanel 默认在 app 不是前台时藏起来，岛要一直在
        ns_window.setHidesOnDeactivate(false);
        ns_window.setOpaque(false);
        ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
        ns_window.setHasShadow(false);
        // Above the menu bar
        ns_window.setLevel(NSStatusWindowLevel + 1);

        let view = ns_window.contentView().ok_or("the window has no view")?;
        view.setWantsLayer(true);
        let layer = view.layer().ok_or("the view has no layer")?;
        debug_assert!(
            !view.isFlipped() && !layer.isGeometryFlipped(),
            "the mask expects y to grow upward"
        );
        // The page shows only where the mask's layers are
        let mask = CALayer::new();
        let shapes = [CALayer::new(), CALayer::new()];
        for shape in &shapes {
            shape.setBackgroundColor(Some(&NSColor::blackColor().CGColor()));
            mask.addSublayer(shape);
        }
        unsafe { layer.setMask(Some(&mask)) };

        let observer = Observer::new(mtm);
        let receiver: &AnyObject = &observer;
        // SAFETY: the observer has a `screensChanged:` method, and stays as long as the app
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                receiver,
                sel!(screensChanged:),
                Some(NSApplicationDidChangeScreenParametersNotification),
                None,
            );
        }
        let area = tracking_area(&view, &observer, NSRect::ZERO);
        let mut island = Self {
            window,
            ns_window,
            view,
            mask,
            shapes,
            observer,
            area,
            screen,
            state: State::default(),
        };
        island.place();
        Ok(island)
    }

    fn set_content(&mut self, content: Option<Content>) {
        if content.is_none() {
            self.ns_window.orderOut(None);
        }
        if self.state.set_content(content) {
            self.open_changed();
        }
        emit(&self.window, CONTENT_EVENT, &self.state.content);
    }

    fn set_open(&mut self, open: bool) {
        if self.state.set_open(open) {
            self.open_changed();
        }
    }

    /// 展开或收起以后：遮罩和追踪区域跟着变，再告诉页面。
    fn open_changed(&mut self) {
        self.draw(true);
        self.track();
        emit(&self.window, OPEN_EVENT, self.state.open);
    }

    fn screens_changed(&mut self, mtm: MainThreadMarker) {
        // With no screen, for example while the screens sleep, it stays where it was
        let Some(screen) = main_screen(mtm) else {
            return;
        };
        self.screen = screen;
        self.place();
        emit(&self.window, LAYOUT_EVENT, self.layout());
    }

    /// Puts the window and the mask on the main screen as it is now, and sets whether the island
    /// shows over full-screen apps on this screen.
    fn place(&mut self) {
        let window = geometry::window(&self.screen);
        self.ns_window.setFrame_display(ns_rect(window), true);
        self.ns_window
            .setCollectionBehavior(collection_behavior(self.screen.notch.is_some()));
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.mask.setFrame(local(window, window));
        CATransaction::commit();
        self.draw(false);
        self.track();
    }

    fn closed(&self) -> Closed {
        Closed::new(&self.screen, self.state.widths.0, self.state.widths.1)
    }

    fn opened(&self) -> Rect {
        geometry::open(
            &self.screen,
            &self.closed(),
            &self.state.panel.unwrap_or_default(),
        )
    }

    /// Moves the mask's layers to the open panel or to the closed shapes.
    fn draw(&self, animated: bool) {
        let window = geometry::window(&self.screen);
        let closed = self.closed();
        let notch = matches!(closed, Closed::Notch { .. });
        let shapes = closed.shapes();
        CATransaction::begin();
        if animated {
            // SAFETY: Core Animation's own constants
            let (seconds, timing) = unsafe {
                if self.state.open {
                    (OPEN_SECONDS, kCAMediaTimingFunctionEaseOut)
                } else {
                    (CLOSE_SECONDS, kCAMediaTimingFunctionEaseInEaseOut)
                }
            };
            CATransaction::setAnimationDuration(seconds);
            CATransaction::setAnimationTimingFunction(Some(
                &CAMediaTimingFunction::functionWithName(timing),
            ));
        } else {
            CATransaction::setDisableActions(true);
        }
        // With a notch the top corners stay square, against the top of the screen
        let corners = if notch {
            CACornerMask::LayerMinXMinYCorner | CACornerMask::LayerMaxXMinYCorner
        } else {
            CACornerMask::LayerMinXMinYCorner
                | CACornerMask::LayerMaxXMinYCorner
                | CACornerMask::LayerMinXMaxYCorner
                | CACornerMask::LayerMaxXMaxYCorner
        };
        for (i, layer) in self.shapes.iter().enumerate() {
            let (frame, radius) = if self.state.open {
                (local(window, self.opened()), OPEN_RADIUS)
            } else {
                match shapes.get(i).or(shapes.last()) {
                    Some(&shape) if notch => (local(window, shape), NOTCH_RADIUS),
                    Some(&shape) => (local(window, shape), shape.height / 2.0),
                    None => (NSRect::ZERO, 0.0),
                }
            };
            layer.setFrame(frame);
            layer.setCornerRadius(radius);
            layer.setMaskedCorners(corners);
        }
        CATransaction::commit();
    }

    /// Makes the tracking area match what shows: the whole panel when open, the shapes when
    /// closed.
    fn track(&mut self) {
        let window = geometry::window(&self.screen);
        let area = if self.state.open {
            Some(self.opened())
        } else {
            self.closed().bounds()
        };
        self.view.removeTrackingArea(&self.area);
        self.area = tracking_area(
            &self.view,
            &self.observer,
            area.map_or(NSRect::ZERO, |area| local(window, area)),
        );
    }

    fn layout(&self) -> Layout {
        let window = geometry::window(&self.screen);
        let closed = self.closed();
        let (left, right) = closed.sides();
        Layout {
            notch: matches!(closed, Closed::Notch { .. }),
            left: left.map(|rect| page_rect(window, rect)),
            right: right.map(|rect| page_rect(window, rect)),
            top_right: page_rect(window, closed.top_right(&self.screen, self.opened())),
            open: page_rect(window, self.opened()),
        }
    }
}

/// Runs `f` on the island. Does nothing when there is no island.
fn with_island<R>(f: impl FnOnce(&mut Island) -> R) -> Option<R> {
    ISLAND.with(|cell| {
        // AppKit sends mouse and screen messages from the event loop, never while this runs.
        // Checked anyway: a panic would end the app
        let Ok(mut island) = cell.try_borrow_mut() else {
            log::warn!("island: already in use");
            return None;
        };
        island.as_mut().map(f)
    })
}

/// 不在主线程上时返回 `None`，并记日志：岛只在主线程上，别的线程看到的岛总是空的。
fn main_thread() -> Option<MainThreadMarker> {
    let mtm = MainThreadMarker::new();
    if mtm.is_none() {
        log::warn!("island: not on the main thread");
    }
    mtm
}

fn on_main_thread(app: &AppHandle, f: impl FnOnce(MainThreadMarker) + Send + 'static) {
    let result = app.run_on_main_thread(move || match MainThreadMarker::new() {
        Some(mtm) => f(mtm),
        None => log::warn!("island: not on the main thread"),
    });
    if let Err(e) = result {
        log::warn!("island: cannot run on the main thread: {e}");
    }
}

/// Returns the main screen: the one with the menu bar. `None` when there is no screen.
fn main_screen(mtm: MainThreadMarker) -> Option<Screen> {
    // The main screen comes first. `NSScreen.mainScreen` is another one: the screen with the key
    // window
    let screen = NSScreen::screens(mtm).firstObject()?;
    let frame = rect(screen.frame());
    let top = screen.safeAreaInsets().top;
    // Only a screen with a notch has a top safe area
    let notch = (top > 0.0).then(|| Notch {
        width: frame.width
            - screen.auxiliaryTopLeftArea().size.width
            - screen.auxiliaryTopRightArea().size.width,
        height: top,
    });
    Some(Screen {
        frame,
        visible_frame: rect(screen.visibleFrame()),
        notch,
    })
}

/// Returns how the island's window shows across desktops: on every desktop, still while the
/// desktops switch, and skipped when the user switches windows.
///
/// The island shows over full-screen apps only on a screen with a notch. There, a full-screen app
/// draws below the notch by default and leaves a black band at the top, where the closed island
/// sits. Without a notch, full-screen content reaches the top of the screen, and the island would
/// cover it.
fn collection_behavior(notch: bool) -> NSWindowCollectionBehavior {
    let always = NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::Stationary
        | NSWindowCollectionBehavior::IgnoresCycle;
    if notch {
        always | NSWindowCollectionBehavior::FullScreenAuxiliary
    } else {
        always
    }
}

/// Adds a tracking area that tells `observer` when the mouse goes into `rect` and out of it.
fn tracking_area(view: &NSView, observer: &Observer, rect: NSRect) -> Retained<NSTrackingArea> {
    let owner: &AnyObject = observer;
    // SAFETY: the observer has `mouseEntered:` and `mouseExited:`, and stays as long as the area
    let area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            rect,
            NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways,
            Some(owner),
            None,
        )
    };
    view.addTrackingArea(&area);
    area
}

/// Returns `rect` in the window's view: from the bottom-left corner of `window`, with y growing
/// upward.
fn local(window: Rect, rect: Rect) -> NSRect {
    NSRect::new(
        NSPoint::new(rect.x - window.x, rect.y - window.y),
        NSSize::new(rect.width, rect.height),
    )
}

/// Returns `rect` in the page of `window`.
fn page_rect(window: Rect, rect: Rect) -> PageRect {
    PageRect {
        x: rect.x - window.x,
        y: window.top() - rect.top(),
        width: rect.width,
        height: rect.height,
    }
}

fn ns_rect(rect: Rect) -> NSRect {
    NSRect::new(
        NSPoint::new(rect.x, rect.y),
        NSSize::new(rect.width, rect.height),
    )
}

fn rect(rect: NSRect) -> Rect {
    Rect {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
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

    #[test]
    fn the_view_and_the_page_count_from_different_corners() {
        // 外接屏上岛的窗口，和右边一个宽 60 的胶囊（见 geometry 的测试）
        let window = Rect {
            x: 700.0,
            y: 858.0,
            width: 520.0,
            height: 219.0,
        };
        let capsule = Rect {
            x: 930.0,
            y: 1058.0,
            width: 60.0,
            height: 19.0,
        };
        // 视图从窗口左下角算：1058 - 858 = 200
        assert_eq!(
            local(window, capsule),
            NSRect::new(NSPoint::new(230.0, 200.0), NSSize::new(60.0, 19.0))
        );
        // 页面从窗口左上角算：胶囊贴着窗口顶边
        assert_eq!(
            page_rect(window, capsule),
            PageRect {
                x: 230.0,
                y: 0.0,
                width: 60.0,
                height: 19.0,
            }
        );
    }
}
