//! 主屏、收起和展开的岛在哪、给页面的布局，还有几种坐标之间的换算：屏幕上从主屏左下角算、y 向上；
//! 视图从窗口左下角算、y 向上；页面从窗口左上角算、y 向下。

use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::geometry::{self, Closed, Notch, Rect, Screen};
use super::Island;
use crate::island::{Layout, PageRect};

impl Island {
    pub(super) fn closed(&self) -> Closed {
        Closed::new(&self.screen, self.state.widths.0, self.state.widths.1)
    }

    pub(super) fn opened(&self) -> Rect {
        geometry::open(
            &self.screen,
            &self.closed(),
            &self.state.panel.unwrap_or_default(),
        )
    }

    pub(super) fn layout(&self) -> Layout {
        let window = geometry::window(&self.screen);
        let closed = self.closed();
        let (left, right) = closed.sides();
        Layout {
            notch: matches!(closed, Closed::Notch { .. }),
            left: left.map(|rect| page_rect(window, rect)),
            right: right.map(|rect| page_rect(window, rect)),
            top_right: page_rect(window, closed.top_right(&self.screen, self.opened())),
            open: page_rect(window, self.opened()),
            drawn: None,
        }
    }
}

/// Returns the main screen: the one with the menu bar. `None` when there is no screen.
pub(super) fn main_screen(mtm: MainThreadMarker) -> Option<Screen> {
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

/// Returns `rect` in the window's view: from the bottom-left corner of `window`, with y growing
/// upward.
pub(super) fn local(window: Rect, rect: Rect) -> NSRect {
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

pub(super) fn ns_rect(rect: Rect) -> NSRect {
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
