//! 遮罩：页面只在遮罩里两个黑色图层的地方显示出来。展开收起时 Core Animation 把图层挪到新的形状上，
//! 动画就是这么来的。

use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSView};
use objc2_foundation::NSRect;
use objc2_quartz_core::{
    kCAMediaTimingFunctionEaseInEaseOut, kCAMediaTimingFunctionEaseOut, CACornerMask, CALayer,
    CAMediaTimingFunction, CATransaction,
};

use super::geometry::{self, Closed, NOTCH_RADIUS};
use super::layout::local;
use super::Island;
use crate::island::OPEN_RADIUS;

/// How long opening takes, in seconds
const OPEN_SECONDS: f64 = 0.32;
/// How long closing takes, in seconds
const CLOSE_SECONDS: f64 = 0.24;

/// 给 `view` 加上遮罩，返回遮罩和它里面的两个黑色图层。
pub(super) fn new(view: &NSView) -> Result<(Retained<CALayer>, [Retained<CALayer>; 2]), String> {
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
    Ok((mask, shapes))
}

impl Island {
    /// Moves the mask's layers to the open panel or to the closed shapes.
    pub(super) fn draw(&self, animated: bool) {
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
}
