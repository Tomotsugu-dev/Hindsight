//! The island's shapes on the main screen, open and closed. Only works them out; does not touch
//! the window.
//!
//! All coordinates are places on the screen: in logical pixels, from the bottom-left corner of the
//! main screen, with y growing upward.

use super::PanelSize;

/// The most the closed island reaches past the notch on each side
const EAR: f64 = 90.0;
/// 展开的岛最宽多少。内容按上限放满时要这么宽：刘海屏上右上角 20 宽的字放在刘海右边
const MAX_OPEN_WIDTH: f64 = 520.0;
/// 展开时下面的面板最高多少：内容按上限放满也放得下
const MAX_PANEL_HEIGHT: f64 = 200.0;
/// With no notch: the gap from the island to the top of the screen, and from the capsules to the
/// bottom of the menu bar
const MENU_BAR_GAP: f64 = 3.0;
/// With no notch: the gap between the two capsules
const CAPSULE_GAP: f64 = 6.0;
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

/// 返回岛的窗口：和展开到最宽、最高时的岛一样大。只有 [`Closed`] 的形状或者 [`open`] 显示出来，
/// 其余是透明的。
pub fn window(screen: &Screen) -> Rect {
    let (width, gap) = match &screen.notch {
        Some(notch) => (MAX_OPEN_WIDTH.max(notch.width + 2.0 * EAR), 0.0),
        // The same gap to the top of the screen as when closed
        None => (MAX_OPEN_WIDTH, MENU_BAR_GAP),
    };
    // AppKit 把窗口放在整数的点上，这里先取整：遮罩按这个窗口画，形状的边才和窗口的边对得上。
    // 刘海高可以是 33.5
    let height = (strip(screen) + MAX_PANEL_HEIGHT).ceil();
    let frame = screen.frame;
    Rect {
        x: (frame.x + (frame.width - width) / 2.0).round(),
        y: frame.top() - gap - height,
        width,
        height,
    }
}

/// 返回展开的岛：最上面一条，加上下面的面板，在屏幕正中。
///
/// - 宽：放得下面板、右上角的字和收起时的形状，最宽到窗口那么宽；
/// - 高：最上面一条加面板。面板太矮的补到圆角放得下，太高的截到 [`MAX_PANEL_HEIGHT`]。
///
/// `panel` 是页面量出来的，经 JSON 传来，不会是 NaN。
pub fn open(screen: &Screen, closed: &Closed, panel: &PanelSize) -> Rect {
    let window = window(screen);
    let center = window.x + window.width / 2.0;
    // 字从刘海或左边胶囊的右边开始，岛又是居中的，所以要的宽度是它到中线距离的两倍
    let top_right = match (panel.top_right, closed.top_right_start()) {
        (Some(text), Some(start)) => 2.0 * (start - center + text),
        (Some(text), None) => text,
        (None, _) => 0.0,
    };
    // 刘海两边的耳朵各按内容宽，收起的形状不一定居中：要盖住它离中线远的那边
    let closed_width = closed.bounds().map_or(0.0, |bounds| {
        2.0 * (center - bounds.x).max(bounds.right() - center)
    });
    let width = panel
        .width
        .max(top_right)
        .max(closed_width)
        .clamp(2.0 * OPEN_RADIUS, window.width);
    let height = strip(screen) + panel.height.clamp(2.0 * OPEN_RADIUS, MAX_PANEL_HEIGHT);
    Rect {
        x: center - width / 2.0,
        y: window.top() - height,
        width,
        height,
    }
}

/// 返回最上面那一条的高度：刘海的高度，没有刘海时是胶囊的高度。
fn strip(screen: &Screen) -> f64 {
    match &screen.notch {
        Some(notch) => notch.height,
        None => menu_bar_height(screen) - 2.0 * MENU_BAR_GAP,
    }
}

/// The closed island.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Closed {
    /// 包着刘海：刘海加两边的耳朵。`left`、`right` 是两边耳朵的宽，没有内容的一边是 0
    Notch { rect: Rect, left: f64, right: f64 },
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
    ///   a side with nothing in it
    pub fn new(screen: &Screen, left: Option<f64>, right: Option<f64>) -> Self {
        let window = window(screen);
        let height = strip(screen);
        if let Some(notch) = &screen.notch {
            // 耳朵和胶囊一样最窄是圆，最宽 EAR
            let ear = |width: Option<f64>| width.map_or(0.0, |width| width.max(height).min(EAR));
            let (left, right) = (ear(left), ear(right));
            // 刘海在屏幕正中。窗口取整以后可能偏半个点，不按它算
            let x = screen.frame.x + (screen.frame.width - notch.width) / 2.0 - left;
            return Closed::Notch {
                rect: Rect {
                    x,
                    y: window.top() - height,
                    width: left + notch.width + right,
                    height,
                },
                left,
                right,
            };
        }
        // At its narrowest a capsule is a circle, never narrower than it is tall. At its widest it
        // is half the island
        let widest = ((window.width - CAPSULE_GAP) / 2.0).max(height);
        let fit = |width: f64| width.max(height).min(widest);
        let (left, right) = (left.map(fit), right.map(fit));
        let gap = if left.is_some() && right.is_some() {
            CAPSULE_GAP
        } else {
            0.0
        };
        let total = left.unwrap_or(0.0) + gap + right.unwrap_or(0.0);
        let x = window.x + (window.width - total) / 2.0;
        let capsule = |x: f64, width: f64| Rect {
            x,
            y: window.top() - height,
            width,
            height,
        };
        Closed::Floating {
            left: left.map(|width| capsule(x, width)),
            right: right.map(|width| capsule(x + left.unwrap_or(0.0) + gap, width)),
        }
    }

    /// Returns where the content of each side goes: beside the notch, or in the capsules.
    pub fn sides(&self) -> (Option<Rect>, Option<Rect>) {
        match *self {
            Closed::Notch { rect, left, right } => (
                (left > 0.0).then_some(Rect {
                    width: left,
                    ..rect
                }),
                (right > 0.0).then_some(Rect {
                    x: rect.right() - right,
                    width: right,
                    ..rect
                }),
            ),
            Closed::Floating { left, right } => (left, right),
        }
    }

    /// 返回展开时右上角的字放在哪：展开的岛 `open` 最上面一条里，从刘海或者左边胶囊的右边到岛的
    /// 右边。左边没有胶囊时整条都可以用。字靠左放，紧跟在左边的内容后面。
    pub fn top_right(&self, screen: &Screen, open: Rect) -> Rect {
        let x = self.top_right_start().unwrap_or(open.x);
        let height = strip(screen);
        Rect {
            x,
            y: open.top() - height,
            width: open.right() - x,
            height,
        }
    }

    /// 右上角的字最左从哪里开始：刘海右边，或者左边胶囊右边。`None`：左边没有胶囊。
    fn top_right_start(&self) -> Option<f64> {
        match *self {
            Closed::Notch { rect, right, .. } => Some(rect.right() - right),
            Closed::Floating { left, .. } => left.map(|left| left.right()),
        }
    }

    /// Returns the shapes, from left to right.
    pub fn shapes(&self) -> Vec<Rect> {
        match *self {
            Closed::Notch { rect, .. } => vec![rect],
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

    fn panel(width: f64, height: f64, top_right: Option<f64>) -> PanelSize {
        PanelSize {
            width,
            height,
            top_right,
        }
    }

    #[test]
    fn wraps_the_notch() {
        // 收起：刘海在 656 到 856，左边伸出 40、右边 60，贴着屏幕顶边
        let closed = Closed::new(&built_in(), Some(40.0), Some(60.0));
        assert_eq!(
            closed,
            Closed::Notch {
                rect: rect(616.0, 950.0, 300.0, 32.0),
                left: 40.0,
                right: 60.0,
            }
        );
        // 展开：居中，盖住收起时离中线 756 远的右边 916：宽 2 * 160；高是刘海加面板：982 - 112 = 870
        assert_eq!(
            open(&built_in(), &closed, &panel(300.0, 80.0, None)),
            rect(596.0, 870.0, 320.0, 112.0)
        );
        // 面板宽 450：还在屏幕正中，756 - 225 = 531
        assert_eq!(
            open(&built_in(), &closed, &panel(450.0, 80.0, None)),
            rect(531.0, 870.0, 450.0, 112.0)
        );
    }

    #[test]
    fn the_window_is_in_whole_points() {
        // 15 寸 MacBook Air 的内置屏幕：刘海高 33.5
        let screen = Screen {
            frame: rect(0.0, 0.0, 1710.0, 1107.0),
            visible_frame: rect(0.0, 55.0, 1710.0, 1018.0),
            notch: Some(Notch {
                width: 185.0,
                height: 33.5,
            }),
        };
        let placed = window(&screen);
        assert_eq!(placed, rect(595.0, 873.0, 520.0, 234.0));
        // 形状还是刘海那么高，贴着窗口和屏幕的顶边
        let shape = Closed::new(&screen, Some(0.0), Some(80.0)).shapes()[0];
        assert_eq!(shape.height, 33.5);
        assert_eq!(shape.top(), placed.top());
        // 屏幕宽减窗口宽是单数：窗口取整，刘海还按屏幕正中算
        let screen = Screen {
            frame: rect(0.0, 0.0, 1511.0, 982.0),
            ..built_in()
        };
        assert_eq!(window(&screen).x, 496.0);
        let shape = Closed::new(&screen, None, None).shapes()[0];
        assert_eq!(shape.x, 655.5);
    }

    #[test]
    fn the_open_island_is_as_wide_as_its_content() {
        let closed = Closed::new(&external(), Some(40.0), Some(100.0));
        let width = |w: f64| open(&external(), &closed, &panel(w, 80.0, None)).width;
        assert_eq!(width(300.0), 300.0);
        // 至少盖住收起时的两个胶囊和中间的空隙
        assert_eq!(width(10.0), 146.0);
        // 最宽到窗口那么宽
        assert_eq!(width(2000.0), 520.0);
    }

    #[test]
    fn the_panel_is_kept_between_the_corners_and_the_window() {
        // 窗口和展开到最宽、最高时的岛一样大：32 + 200
        let window = window(&built_in());
        assert_eq!(window, rect(496.0, 750.0, 520.0, 232.0));
        let closed = Closed::new(&built_in(), None, None);
        assert_eq!(
            open(&built_in(), &closed, &panel(2000.0, 1000.0, None)),
            window
        );
        // 太矮：补到圆角放得下
        for height in [0.0, -5.0] {
            assert_eq!(
                open(&built_in(), &closed, &panel(300.0, height, None)).height,
                32.0 + 2.0 * OPEN_RADIUS,
                "{height}"
            );
        }
    }

    #[test]
    fn the_text_at_the_top_right_goes_right_of_the_notch() {
        // 刘海右边 856 到岛的右边 1016
        let closed = Closed::new(&built_in(), None, None);
        let open = window(&built_in());
        assert_eq!(
            closed.top_right(&built_in(), open),
            rect(856.0, 950.0, 160.0, 32.0)
        );
        // 没有刘海：从左边胶囊的右边 927 到岛的右边 1220
        let closed = Closed::new(&external(), Some(40.0), Some(100.0));
        let open = window(&external());
        assert_eq!(
            closed.top_right(&external(), open),
            rect(927.0, 1058.0, 293.0, 19.0)
        );
        // 没有左边的胶囊，整条都用
        let closed = Closed::new(&external(), None, Some(100.0));
        assert_eq!(closed.top_right(&external(), open).x, 700.0);
    }

    #[test]
    fn the_island_opens_wide_enough_for_the_text_at_the_top_right() {
        // 刘海右边到中线 100，字要 150：岛两边各 250
        let closed = Closed::new(&built_in(), None, None);
        let opened = open(&built_in(), &closed, &panel(100.0, 80.0, Some(150.0)));
        assert_eq!(opened.width, 500.0);
        assert_eq!(closed.top_right(&built_in(), opened).width, 150.0);
        // 没有刘海：左边胶囊的右边 927 在中线 960 左边 33
        let closed = Closed::new(&external(), Some(40.0), Some(100.0));
        let opened = open(&external(), &closed, &panel(100.0, 80.0, Some(200.0)));
        assert_eq!(opened.width, 2.0 * (200.0 - 33.0));
        assert_eq!(closed.top_right(&external(), opened).width, 200.0);
    }

    #[test]
    fn the_content_goes_beside_the_notch() {
        // 刘海在 656 到 856 之间。只放图标的一边和刘海一样高，最窄是圆；内容宽的一边最多伸出 90
        let (left, right) = Closed::new(&built_in(), Some(0.0), Some(500.0)).sides();
        assert_eq!(left, Some(rect(624.0, 950.0, 32.0, 32.0)));
        assert_eq!(right, Some(rect(856.0, 950.0, 90.0, 32.0)));
        // 没有内容的一边不伸出去
        let closed = Closed::new(&built_in(), None, Some(70.0));
        assert_eq!(closed.sides().0, None);
        assert_eq!(closed.shapes()[0], rect(656.0, 950.0, 270.0, 32.0));
        // 没有刘海时，内容就在胶囊里
        let closed = Closed::new(&external(), None, Some(60.0));
        assert_eq!(closed.sides(), (None, Some(closed.shapes()[0])));
    }

    #[test]
    fn floats_without_a_notch() {
        // 在屏幕正中，离屏幕顶 3；高是胶囊的 19 加面板：978 + 99 + 3 = 1080
        let closed = Closed::new(&external(), None, Some(60.0));
        assert_eq!(
            open(&external(), &closed, &panel(300.0, 80.0, None)),
            rect(810.0, 978.0, 300.0, 99.0)
        );
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
        let window = window(&external());
        assert_eq!(bounds.x, window.x);
        assert_eq!(bounds.right(), window.right());
    }

    #[test]
    fn a_hidden_menu_bar_uses_the_default_height() {
        let mut screen = external();
        screen.visible_frame = screen.frame;
        // 按 30 算：胶囊高 30 - 6，展开高 24 加面板
        let closed = Closed::new(&screen, Some(40.0), None);
        assert_eq!(closed.shapes()[0].height, 24.0);
        assert_eq!(
            open(&screen, &closed, &panel(300.0, 80.0, None)).height,
            104.0
        );
    }

    #[test]
    fn corners_are_at_most_half_the_shorter_side() {
        // 圆角超过短边的一半，形状就画坏了
        let notch = Closed::new(&built_in(), None, None).shapes()[0];
        assert!(NOTCH_RADIUS <= notch.height / 2.0);
        // 最小的展开：面板什么都没量出来，只有一个圆形胶囊
        for screen in [built_in(), external()] {
            let closed = Closed::new(&screen, Some(0.0), None);
            let open = open(&screen, &closed, &PanelSize::default());
            assert!(OPEN_RADIUS <= open.width.min(open.height) / 2.0);
        }
    }
}
