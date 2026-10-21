//! The island's shapes on the main screen, open and closed. Only works them out; does not touch
//! the window.
//!
//! All coordinates are places on the screen: in logical pixels, from the bottom-left corner of the
//! main screen, with y growing upward.

/// How far the island reaches past the notch on each side
const EAR: f64 = 90.0;
/// How much taller the island is open than closed
const OPEN_EXTRA_HEIGHT: f64 = 96.0;
/// With no notch: the gap from the island to the top of the screen, and from the capsules to the
/// bottom of the menu bar
const MENU_BAR_GAP: f64 = 3.0;
/// With no notch: the gap between the two capsules
const CAPSULE_GAP: f64 = 6.0;
/// With no notch: the width of the open island
const FLOATING_WIDTH: f64 = 336.0;
/// Used when the menu bar hides itself and its height cannot be measured
const DEFAULT_MENU_BAR_HEIGHT: f64 = 30.0;
/// The radius of the two bottom corners around the notch
pub const NOTCH_RADIUS: f64 = 10.0;
/// The corner radius of the open island
pub const OPEN_RADIUS: f64 = 16.0;

/// `x` and `y` are the bottom-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn top(&self) -> f64 {
        self.y + self.height
    }
}

/// What the island needs to know about the main screen.
pub struct Screen {
    /// The whole screen, with the menu bar and the Dock
    pub frame: Rect,
    /// The screen without the menu bar and the Dock
    pub visible_frame: Rect,
    /// `None` when there is no notch
    pub notch: Option<Notch>,
}

pub struct Notch {
    pub width: f64,
    pub height: f64,
}

/// Returns the open island. The window is this size: when closed, only the shapes of [`Closed`]
/// show, and the rest is clear.
pub fn open(screen: &Screen) -> Rect {
    let (width, height, gap) = match &screen.notch {
        Some(notch) => (
            notch.width + 2.0 * EAR,
            notch.height + OPEN_EXTRA_HEIGHT,
            0.0,
        ),
        // The same gap to the top of the screen as when closed
        None => (
            FLOATING_WIDTH,
            menu_bar_height(screen) - MENU_BAR_GAP + OPEN_EXTRA_HEIGHT,
            MENU_BAR_GAP,
        ),
    };
    let frame = screen.frame;
    Rect {
        x: frame.x + (frame.width - width) / 2.0,
        y: frame.top() - gap - height,
        width,
        height,
    }
}

/// The closed island.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Closed {
    /// Around the notch: the top strip of the open island
    Notch(Rect),
    /// Two capsules as wide as their content; `None` on a side with nothing in it
    Floating {
        left: Option<Rect>,
        right: Option<Rect>,
    },
}

impl Closed {
    /// Returns the closed island.
    ///
    /// - `screen`: the main screen
    /// - `left`, `right`: the widths of the content on each side, measured by the page. `None` for
    ///   a side with nothing in it. Not used with a notch
    pub fn new(screen: &Screen, left: Option<f64>, right: Option<f64>) -> Self {
        let open = open(screen);
        if let Some(notch) = &screen.notch {
            return Closed::Notch(Rect {
                y: open.top() - notch.height,
                height: notch.height,
                ..open
            });
        }
        let height = menu_bar_height(screen) - 2.0 * MENU_BAR_GAP;
        // At its narrowest a capsule is a circle, never narrower than it is tall. At its widest it
        // is half the island
        let widest = ((open.width - CAPSULE_GAP) / 2.0).max(height);
        let fit = |width: f64| width.max(height).min(widest);
        let (left, right) = (left.map(fit), right.map(fit));
        let gap = if left.is_some() && right.is_some() {
            CAPSULE_GAP
        } else {
            0.0
        };
        let total = left.unwrap_or(0.0) + gap + right.unwrap_or(0.0);
        let x = open.x + (open.width - total) / 2.0;
        let capsule = |x: f64, width: f64| Rect {
            x,
            y: open.top() - height,
            width,
            height,
        };
        Closed::Floating {
            left: left.map(|width| capsule(x, width)),
            right: right.map(|width| capsule(x + left.unwrap_or(0.0) + gap, width)),
        }
    }

    /// Returns the shapes, from left to right.
    pub fn shapes(&self) -> Vec<Rect> {
        match *self {
            Closed::Notch(rect) => vec![rect],
            Closed::Floating { left, right } => left.into_iter().chain(right).collect(),
        }
    }

    /// Returns the smallest rectangle around all the shapes, with the gap between the capsules.
    /// `None` when there are no shapes.
    pub fn bounds(&self) -> Option<Rect> {
        let shapes = self.shapes();
        let leftmost = shapes.first()?;
        let rightmost = shapes.last()?;
        Some(Rect {
            width: rightmost.right() - leftmost.x,
            ..*leftmost
        })
    }
}

/// Returns the height of the menu bar. When the menu bar hides itself, the visible part reaches the
/// top of the screen, and the measured height is 0.
fn menu_bar_height(screen: &Screen) -> f64 {
    let measured = screen.frame.top() - screen.visible_frame.top();
    if measured > 2.0 * MENU_BAR_GAP {
        measured
    } else {
        DEFAULT_MENU_BAR_HEIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// 14 寸 MacBook 的内置屏幕：刘海宽 200、高 32
    fn built_in() -> Screen {
        Screen {
            frame: rect(0.0, 0.0, 1512.0, 982.0),
            visible_frame: rect(0.0, 0.0, 1512.0, 950.0),
            notch: Some(Notch {
                width: 200.0,
                height: 32.0,
            }),
        }
    }

    /// 外接屏，菜单栏高 25
    fn external() -> Screen {
        Screen {
            frame: rect(0.0, 0.0, 1920.0, 1080.0),
            visible_frame: rect(0.0, 0.0, 1920.0, 1055.0),
            notch: None,
        }
    }

    #[test]
    fn wraps_the_notch() {
        // 刘海加两边各 90，贴着屏幕顶边居中：854 + 128 = 982
        assert_eq!(open(&built_in()), rect(566.0, 854.0, 380.0, 128.0));
        // 收起时是最上面一条，用不到内容宽度
        assert_eq!(
            Closed::new(&built_in(), Some(40.0), Some(60.0)),
            Closed::Notch(rect(566.0, 950.0, 380.0, 32.0))
        );
    }

    #[test]
    fn floats_without_a_notch() {
        // 离屏幕顶 3：959 + 118 + 3 = 1080
        assert_eq!(open(&external()), rect(792.0, 959.0, 336.0, 118.0));
        // 页面量出内容之前，没有胶囊
        assert_eq!(Closed::new(&external(), None, None).bounds(), None);
    }

    #[test]
    fn the_capsules_take_the_measured_widths() {
        let closed = Closed::new(&external(), Some(40.0), Some(100.0));
        // 40、空隙 6、100，一起在屏幕正中；菜单栏高 25，上下各留 3：胶囊高 19
        assert_eq!(
            closed,
            Closed::Floating {
                left: Some(rect(887.0, 1058.0, 40.0, 19.0)),
                right: Some(rect(933.0, 1058.0, 100.0, 19.0)),
            }
        );
        // 空隙也算岛
        assert_eq!(closed.bounds(), Some(rect(887.0, 1058.0, 146.0, 19.0)));
    }

    #[test]
    fn one_side_alone_is_centered() {
        assert_eq!(
            Closed::new(&external(), None, Some(60.0)),
            Closed::Floating {
                left: None,
                right: Some(rect(930.0, 1058.0, 60.0, 19.0)),
            }
        );
    }

    #[test]
    fn the_narrowest_capsule_is_a_circle() {
        // 页面报上来的宽度比高度还窄，甚至是 0、负数：都补成圆
        for width in [-5.0, 0.0, 10.0, 19.0] {
            for shape in Closed::new(&external(), Some(width), Some(width)).shapes() {
                assert_eq!(shape.width, shape.height, "{width}");
            }
        }
    }

    #[test]
    fn capsules_wider_than_the_island_are_cut_to_fit() {
        let bounds = Closed::new(&external(), Some(1000.0), Some(1000.0))
            .bounds()
            .unwrap();
        let open = open(&external());
        assert_eq!(bounds.x, open.x);
        assert_eq!(bounds.right(), open.right());
    }

    #[test]
    fn a_hidden_menu_bar_uses_the_default_height() {
        let mut screen = external();
        screen.visible_frame = screen.frame;
        // 按 30 算：胶囊高 30 - 6，展开高 30 - 3 + 96
        let closed = Closed::new(&screen, Some(40.0), None);
        assert_eq!(closed.shapes()[0].height, 24.0);
        assert_eq!(open(&screen).height, 123.0);
    }

    #[test]
    fn corners_are_at_most_half_the_shorter_side() {
        // 圆角超过短边的一半，形状就画坏了
        let notch = Closed::new(&built_in(), None, None).shapes()[0];
        assert!(NOTCH_RADIUS <= notch.height / 2.0);
        for screen in [built_in(), external()] {
            let open = open(&screen);
            assert!(OPEN_RADIUS <= open.width.min(open.height) / 2.0);
        }
    }
}
