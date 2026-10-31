//! 鼠标移入移出岛、屏幕变化：AppKit 告诉 [`Observer`]，它再告诉岛。别的 app 在前台时也收得到。

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSEvent, NSTrackingArea,
    NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject, NSRect};

use super::geometry;
use super::layout::local;
use super::{with_island, Island};

define_class!(
    // Gets the mouse going over the island and leaving it, and changes of the screens
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "HindsightIslandObserver"]
    pub(super) struct Observer;

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

/// 建好 [`Observer`]，让它收屏幕变化的通知，再在 `view` 上加一个空的追踪区域。
pub(super) fn observe(
    view: &NSView,
    mtm: MainThreadMarker,
) -> (Retained<Observer>, Retained<NSTrackingArea>) {
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
    let area = tracking_area(view, &observer, NSRect::ZERO);
    (observer, area)
}

impl Island {
    /// Makes the tracking area match what shows: the whole panel when open, the shapes when
    /// closed.
    pub(super) fn track(&mut self) {
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
