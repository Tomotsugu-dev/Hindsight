//! Windows 上的岛的位置和大小：一个可以放在屏幕上任何地方、也可以吸附在任务栏上的胶囊，鼠标移上去
//! 展开。只算，不碰窗口。设计见 docs/internal/island-windows.md。
//!
//! 窗口一直是展开到最大时那么大，窗口区域只露出收起的胶囊或者展开的岛：展开收起不改窗口大小。改大小
//! 再往上挪窗口时，WebView2 会空一帧，胶囊闪一下（试验窗口实测）。
//!
//! 屏幕上的位置是物理像素，y 向下；页面里的位置是逻辑像素，从窗口左上角算。

use crate::island::{
    Drawn, Layout, PageRect, PanelSize, MAX_OPEN_WIDTH, MAX_PANEL_HEIGHT, OPEN_RADIUS,
};

/// 胶囊多高，逻辑像素：放得进 Windows 11 默认 48 高的任务栏，上下还各留 8
pub const CAPSULE_HEIGHT: f64 = 32.0;
/// 窗口多大，逻辑像素：展开到最宽、最高时的岛
pub const WINDOW: (f64, f64) = (MAX_OPEN_WIDTH, CAPSULE_HEIGHT + MAX_PANEL_HEIGHT);
/// 第一次出现时，胶囊离工作区右边、下边多远，逻辑像素
const MARGIN: f64 = 16.0;

/// `x`、`y` 是左上角。
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

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    fn contains(&self, (x, y): (f64, f64)) -> bool {
        self.x <= x && x < self.right() && self.y <= y && y < self.bottom()
    }
}

/// 胶囊放在某处时，窗口、页面和窗口区域各是什么样。
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// 窗口在屏幕上
    pub window: Rect,
    /// 胶囊在屏幕上，已经放进了屏幕
    pub capsule: Rect,
    /// 窗口区域，窗口里的物理像素：收起时只露出胶囊，展开时露出整个岛
    pub closed: Rect,
    pub open: Rect,
    /// 页面里各处放在哪
    pub layout: Layout,
}

/// 收起时两边各多宽，逻辑像素。`left`、`right` 是页面量出来的，`None`：那边没东西；0：只放一个
/// 图标，占一个圆。一边最宽是岛的一半。
fn sides(left: Option<f64>, right: Option<f64>) -> (f64, f64) {
    let fit = |width: Option<f64>| {
        width.map_or(0.0, |width| {
            width.clamp(CAPSULE_HEIGHT, MAX_OPEN_WIDTH / 2.0)
        })
    };
    (fit(left), fit(right))
}

/// 收起的胶囊多宽，逻辑像素：两边加起来，最窄是圆。
pub fn capsule_width(left: Option<f64>, right: Option<f64>) -> f64 {
    let (left, right) = sides(left, right);
    (left + right).max(CAPSULE_HEIGHT)
}

/// 第一次出现时胶囊的左上角：工作区右下角，任务栏正上方。
///
/// - `work`：主屏的工作区，物理像素
/// - `width`：胶囊多宽，逻辑像素
pub fn default_at(work: Rect, scale: f64, width: f64) -> (f64, f64) {
    (
        work.right() - (width + MARGIN) * scale,
        work.bottom() - (CAPSULE_HEIGHT + MARGIN / 2.0) * scale,
    )
}

/// 往上展开：胶囊在屏幕下半部分，比如吸附在 Windows 11 的任务栏上。
pub fn opens_up(capsule: Rect, screen: Rect) -> bool {
    capsule.center().1 > screen.y + screen.height / 2.0
}

/// 把胶囊左上角放在 `at`。
///
/// - `screen`：胶囊所在那块屏。胶囊和窗口都放进屏幕里，靠屏幕左右边时展开的岛往屏幕里挪
/// - `up`：往上展开，见 [`opens_up`]
/// - `scale`：这块屏的缩放比例
/// - `widths`、`panel`：页面量出的收起时两边的宽度、展开内容的大小，逻辑像素
pub fn frame(
    at: (f64, f64),
    screen: Rect,
    up: bool,
    scale: f64,
    widths: (Option<f64>, Option<f64>),
    panel: &PanelSize,
) -> Frame {
    let (left, right) = sides(widths.0, widths.1);
    let width = capsule_width(widths.0, widths.1);
    let (window_width, window_height) = WINDOW;
    let capsule = Rect {
        x: at.0.min(screen.right() - width * scale).max(screen.x),
        y: at
            .1
            .min(screen.bottom() - CAPSULE_HEIGHT * scale)
            .max(screen.y),
        width: width * scale,
        height: CAPSULE_HEIGHT * scale,
    };
    // 窗口横着放进屏幕：靠屏幕右边时窗口往左挪，胶囊在窗口里靠右
    let window = Rect {
        x: capsule
            .x
            .min(screen.right() - window_width * scale)
            .max(screen.x),
        y: if up {
            capsule.bottom() - window_height * scale
        } else {
            capsule.y
        },
        width: window_width * scale,
        height: window_height * scale,
    };

    // 以下是页面里的逻辑像素。胶囊在窗口最上面，往上展开时在最下面
    let x = (capsule.x - window.x) / scale;
    let y = if up {
        window_height - CAPSULE_HEIGHT
    } else {
        0.0
    };
    // 展开的岛：放得下面板、收起的胶囊，和左边内容后面跟着的右上角的字
    let top_right = panel.top_right.map_or(0.0, |text| left + text);
    let open_width = panel
        .width
        .max(top_right)
        .max(width)
        .clamp(2.0 * OPEN_RADIUS, window_width);
    let open_height = CAPSULE_HEIGHT + panel.height.clamp(2.0 * OPEN_RADIUS, MAX_PANEL_HEIGHT);
    // 往右放不下就往左挪：窗口的右边就是屏幕的右边
    let open_x = x.min(window_width - open_width);
    let open = PageRect {
        x: open_x,
        y: if up { window_height - open_height } else { 0.0 },
        width: open_width,
        height: open_height,
    };
    let side = |x: f64, width: f64| {
        (width > 0.0).then_some(PageRect {
            x,
            y,
            width,
            height: CAPSULE_HEIGHT,
        })
    };
    let closed = PageRect {
        x,
        y,
        width,
        height: CAPSULE_HEIGHT,
    };
    let layout = Layout {
        notch: false,
        left: side(x, left),
        right: side(x + left, right),
        // 字紧跟在左边的内容后面，到岛的右边
        top_right: PageRect {
            x: open_x + left,
            y,
            width: open_width - left,
            height: CAPSULE_HEIGHT,
        },
        open,
        drawn: Some(Drawn {
            capsule: closed,
            open_left: side(open_x, left),
            up,
        }),
    };
    Frame {
        window,
        capsule,
        closed: physical(closed, scale),
        open: physical(open, scale),
        layout,
    }
}

/// 页面里的矩形换成窗口里的物理像素，往外取整：圆角边上的半透明像素不被切掉。
fn physical(rect: PageRect, scale: f64) -> Rect {
    let x = (rect.x * scale).floor();
    let y = (rect.y * scale).floor();
    Rect {
        x,
        y,
        width: ((rect.x + rect.width) * scale).ceil() - x,
        height: ((rect.y + rect.height) * scale).ceil() - y,
    }
}

/// 胶囊中心落在某条任务栏上时，返回吸附后胶囊的左上角：在任务栏里垂直居中，左右不出任务栏。
/// 竖着的任务栏（Windows 10 可以放在两边）放不下胶囊，不吸附。
pub fn snap(capsule: Rect, taskbars: &[Rect]) -> Option<(f64, f64)> {
    let center = capsule.center();
    let bar = taskbars
        .iter()
        .find(|bar| bar.width > bar.height && bar.contains(center))?;
    Some((
        capsule.x.min(bar.right() - capsule.width).max(bar.x),
        bar.y + (bar.height - capsule.height) / 2.0,
    ))
}

/// 任务栏露在屏幕上：自动隐藏缩下去时只剩一两个像素在屏幕里。
pub fn taskbar_shown(bar: Rect, screen: Rect) -> bool {
    bar.bottom().min(screen.bottom()) - bar.y.max(screen.y) >= bar.height / 2.0
}

/// 前台窗口全屏：没有标题栏，盖满整块屏。最大化的普通窗口有标题栏，不算。
pub fn covers(window: Rect, screen: Rect, caption: bool) -> bool {
    !caption
        && window.x <= screen.x
        && window.y <= screen.y
        && window.right() >= screen.right()
        && window.bottom() >= screen.bottom()
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

    fn page(x: f64, y: f64, width: f64, height: f64) -> PageRect {
        PageRect {
            x,
            y,
            width,
            height,
        }
    }

    /// 2560×1440 的屏，125% 缩放，底下 60 高的任务栏
    fn screen() -> Rect {
        rect(0.0, 0.0, 2560.0, 1440.0)
    }

    fn panel(width: f64, height: f64, top_right: Option<f64>) -> PanelSize {
        PanelSize {
            width,
            height,
            top_right,
        }
    }

    #[test]
    fn a_side_is_at_least_a_circle_and_at_most_half_the_island() {
        // 图标量出来是 0：占一个圆
        assert_eq!(capsule_width(Some(0.0), None), CAPSULE_HEIGHT);
        assert_eq!(capsule_width(Some(0.0), Some(70.0)), 32.0 + 70.0);
        assert_eq!(capsule_width(Some(400.0), None), 260.0);
        // 两边都没东西也画个圆
        assert_eq!(capsule_width(None, None), CAPSULE_HEIGHT);
    }

    #[test]
    fn first_shows_above_the_taskbar_at_the_right() {
        // 工作区底在 1380（任务栏 60 高），胶囊宽 102
        let at = default_at(rect(0.0, 0.0, 2560.0, 1380.0), 1.25, 102.0);
        assert_eq!(at, (2560.0 - 118.0 * 1.25, 1380.0 - 40.0 * 1.25));
    }

    #[test]
    fn opens_down_in_the_top_half_and_up_in_the_bottom_half() {
        let capsule = |y: f64| rect(100.0, y, 128.0, 40.0);
        assert!(!opens_up(capsule(300.0), screen()));
        assert!(opens_up(capsule(1390.0), screen()));
    }

    #[test]
    fn opens_down_from_the_capsule() {
        let frame = frame(
            (500.0, 300.0),
            screen(),
            false,
            1.25,
            (Some(0.0), Some(70.0)),
            &panel(280.0, 100.0, Some(90.0)),
        );
        // 窗口左上角就是胶囊左上角：520×232 逻辑像素
        assert_eq!(frame.window, rect(500.0, 300.0, 650.0, 290.0));
        assert_eq!(frame.capsule, rect(500.0, 300.0, 102.0 * 1.25, 40.0));
        let layout = frame.layout;
        assert_eq!(layout.left, Some(page(0.0, 0.0, 32.0, 32.0)));
        assert_eq!(layout.right, Some(page(32.0, 0.0, 70.0, 32.0)));
        // 展开：宽是面板的 280，高是胶囊加面板
        assert_eq!(layout.open, page(0.0, 0.0, 280.0, 132.0));
        assert_eq!(layout.top_right, page(32.0, 0.0, 248.0, 32.0));
        let drawn = layout.drawn.unwrap();
        assert!(!drawn.up);
        assert_eq!(drawn.capsule, page(0.0, 0.0, 102.0, 32.0));
        // 没往左挪：展开时左边的内容不动
        assert_eq!(drawn.open_left, layout.left);
        // 窗口区域：物理像素
        assert_eq!(frame.closed, rect(0.0, 0.0, 128.0, 40.0));
        assert_eq!(frame.open, rect(0.0, 0.0, 350.0, 165.0));
    }

    #[test]
    fn opens_up_from_a_capsule_on_the_taskbar() {
        let frame = frame(
            (600.0, 1390.0),
            screen(),
            true,
            1.25,
            (Some(0.0), Some(70.0)),
            &panel(280.0, 100.0, None),
        );
        // 窗口的底边是胶囊的底边
        assert_eq!(frame.window, rect(600.0, 1430.0 - 290.0, 650.0, 290.0));
        let layout = frame.layout;
        // 胶囊在窗口最下面，展开的岛的底边也在那里
        assert_eq!(layout.left, Some(page(0.0, 200.0, 32.0, 32.0)));
        assert_eq!(layout.open, page(0.0, 100.0, 280.0, 132.0));
        assert!(layout.drawn.unwrap().up);
        assert_eq!(frame.closed, rect(0.0, 250.0, 128.0, 40.0));
        assert_eq!(frame.open, rect(0.0, 125.0, 350.0, 165.0));
    }

    #[test]
    fn near_the_right_edge_the_open_island_moves_left() {
        // 胶囊贴着屏幕右边：宽 102 逻辑像素
        let frame = frame(
            (2560.0 - 102.0 * 1.25, 1390.0),
            screen(),
            true,
            1.25,
            (Some(0.0), Some(70.0)),
            &panel(280.0, 100.0, Some(150.0)),
        );
        // 窗口右边是屏幕右边，胶囊在窗口里靠右
        assert_eq!(frame.window.right(), 2560.0);
        let layout = frame.layout;
        let drawn = layout.drawn.unwrap();
        assert_eq!(drawn.capsule.x, 520.0 - 102.0);
        // 展开的岛按面板宽 280，放得下左边的 32 加右上角的 150；右边贴着窗口右边
        assert_eq!(layout.open.width, 280.0);
        assert_eq!(layout.open.x, 520.0 - 280.0);
        // 左边的内容跟着岛的左边走，字紧跟在后面
        assert_eq!(drawn.open_left, Some(page(240.0, 200.0, 32.0, 32.0)));
        assert_eq!(layout.top_right, page(272.0, 200.0, 248.0, 32.0));
    }

    #[test]
    fn the_capsule_stays_on_the_screen() {
        let frame = frame(
            (-500.0, -50.0),
            screen(),
            false,
            1.0,
            (Some(60.0), None),
            &PanelSize::default(),
        );
        assert_eq!(frame.capsule, rect(0.0, 0.0, 60.0, 32.0));
        assert_eq!(frame.window.x, 0.0);
    }

    #[test]
    fn snaps_to_the_middle_of_the_taskbar_it_is_dropped_on() {
        let taskbar = rect(0.0, 1380.0, 2560.0, 60.0);
        // 中心落在任务栏上：垂直居中
        let dropped = rect(600.0, 1370.0, 128.0, 40.0);
        assert_eq!(snap(dropped, &[taskbar]), Some((600.0, 1390.0)));
        // 左右不出任务栏
        let past = rect(2450.0, 1400.0, 128.0, 40.0);
        assert_eq!(snap(past, &[taskbar]), Some((2560.0 - 128.0, 1390.0)));
        // 中心没落在任务栏上：不吸附
        assert_eq!(snap(rect(600.0, 1300.0, 128.0, 40.0), &[taskbar]), None);
        // 竖着的任务栏不吸附
        let side = rect(0.0, 0.0, 62.0, 1440.0);
        assert_eq!(snap(rect(0.0, 500.0, 128.0, 40.0), &[side]), None);
    }

    #[test]
    fn a_hidden_taskbar_is_almost_off_the_screen() {
        assert!(taskbar_shown(rect(0.0, 1380.0, 2560.0, 60.0), screen()));
        // 自动隐藏缩下去：只剩 2 个像素
        assert!(!taskbar_shown(rect(0.0, 1438.0, 2560.0, 60.0), screen()));
    }

    #[test]
    fn full_screen_covers_the_screen_without_a_caption() {
        assert!(covers(screen(), screen(), false));
        // 最大化的窗口有标题栏，边框还伸出屏幕几个像素：不算全屏
        assert!(!covers(rect(-8.0, -8.0, 2576.0, 1396.0), screen(), true));
        assert!(!covers(rect(0.0, 0.0, 1280.0, 1440.0), screen(), false));
    }
}
