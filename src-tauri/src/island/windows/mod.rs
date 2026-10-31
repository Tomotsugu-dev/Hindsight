//! The island window on Windows: a black capsule that floats anywhere on the screen, or sits on
//! the taskbar, and opens when the mouse rests on it. 设计和试验窗口的实测见
//! docs/internal/island-windows.md。
//!
//! - 窗口不激活（`WS_EX_NOACTIVATE`）：点岛不会让 Hindsight 变成前台，用户正在用的程序还在前台。
//! - 窗口一直是展开到最大时那么大，窗口区域（`SetWindowRgn`）只露出胶囊或者展开的岛，区域外的点击
//!   穿透到下面的窗口。形状和展开收起的动画由页面画。
//! - 鼠标移入移出由页面告诉这里（[`hover`]）；按住胶囊拖动时，这里跟着光标挪窗口（[`drag()`]）。
//! - 前台程序全屏，或者吸附着的任务栏缩下去了，就藏起来。
//!
//! 这个文件管岛上的内容和展开收起，别的一个功能一个文件：
//! - `geometry.rs`：位置和大小怎么算，只算不碰窗口
//! - `window.rs`：摆窗口、窗口区域、显示和隐藏
//! - `drag.rs`：拖动、吸附到任务栏上
//! - `watch.rs`：盯着全屏和任务栏
//! - `screen.rs`：屏幕和任务栏在哪
//! - `saved.rs`：存下来的位置
//!
//! 岛只在主线程上：页面的同步命令在主线程上执行，后台线程有事都交给主线程做。
//!
//! When the content changes, as on macOS:
//! 1. this sends it to the page;
//! 2. the page draws it, measures it and calls [`measured`], which places the window and returns
//!    where each part goes;
//! 3. the page puts the content there and calls [`ready`]. Only then does the window show.

mod drag;
mod geometry;
mod saved;
mod screen;
mod watch;
mod window;

use std::cell::RefCell;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewWindow};
use winapi::shared::windef::HWND;

use super::content::Content;
use super::{emit, new_window, Layout, PanelSize, State, CONTENT_EVENT, OPEN_EVENT};
use geometry::{Frame, WINDOW};

/// 等页面放完收起的动画（240ms）再缩小窗口区域
const CLOSING: Duration = Duration::from_millis(280);

struct Island {
    window: WebviewWindow,
    hwnd: HWND,
    state: State,
    /// 胶囊左上角在屏幕上的位置，物理像素。`None`：用户还没拖过，放在默认位置
    at: Option<(f64, f64)>,
    /// 吸附在任务栏上
    docked: bool,
    /// 上次摆放的结果，和摆放时那块屏的缩放比例
    frame: Option<(Frame, f64)>,
    /// 页面已经把这份内容放好了
    ready: bool,
    /// 因为前台全屏、吸附着的任务栏缩下去了而藏着
    hidden: bool,
    /// 窗口显示过：第一次经过 tao，之后只用原生的 ShowWindow
    shown: bool,
    dragging: bool,
    /// 每次展开收起都加一：收起后等动画的那段时间里又展开了，就不缩小区域
    turns: u64,
}

thread_local! {
    static ISLAND: RefCell<Option<Island>> = const { RefCell::new(None) };
}

/// Shows `content` on the island in place of what is there. Opens the window the first time.
pub fn show(app: &AppHandle, content: Content) {
    let handle = app.clone();
    on_main_thread(app, move || {
        ISLAND.with(|cell| {
            let Ok(mut island) = cell.try_borrow_mut() else {
                log::warn!("island: already in use");
                return;
            };
            if island.is_none() {
                match Island::new(&handle) {
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
    on_main_thread(app, || {
        with_island(|island| island.set_content(None));
    });
}

/// Returns what is on the island now, for the page when it loads. 要在主线程上调用。
pub fn content() -> Option<Content> {
    with_island(|island| island.state.content.clone()).flatten()
}

/// 收下页面量出的收起两边的宽度和展开内容的大小，摆好窗口，返回各处放在哪。`None`：没有岛。要在
/// 主线程上调用。
pub fn measured(left: Option<f64>, right: Option<f64>, panel: Option<PanelSize>) -> Option<Layout> {
    with_island(|island| {
        island.state.measured(left, right, panel);
        island.place();
        island.frame.as_ref().map(|(frame, _)| frame.layout)
    })
    .flatten()
}

/// Shows the window: the page has put the content in place. Does nothing when nothing is on the
/// island. 要在主线程上调用。
pub fn ready() {
    with_island(|island| {
        if island.state.content.is_some() {
            island.ready = true;
            island.show_or_hide();
        }
    });
}

/// 页面报告鼠标移进了岛（`true`）或者移出去了：移进来展开，移出去收起。要在主线程上调用。
pub fn hover(inside: bool) {
    with_island(|island| island.hover(inside));
}

/// 页面报告在按钮以外的地方按下了鼠标：跟着光标挪岛，松手时决定吸附还是浮动。没怎么动就是点击，
/// 由页面处理。要在主线程上调用。
pub fn drag() {
    with_island(Island::drag);
}

impl Island {
    fn new(app: &AppHandle) -> Result<Self, String> {
        let (width, height) = WINDOW;
        let window = new_window(app, |builder| {
            builder
                .inner_size(width, height)
                .transparent(true)
                // WS_EX_NOACTIVATE：点了不激活
                .focusable(false)
                .always_on_top(true)
        })?;
        let hwnd = match window.hwnd() {
            Ok(hwnd) => hwnd.0 as HWND,
            Err(e) => {
                // Or the next try would fail too: the label is taken
                let _ = window.destroy();
                return Err(e.to_string());
            }
        };
        watch::start(app);
        let saved = saved::load();
        Ok(Self {
            window,
            hwnd,
            state: State::default(),
            at: saved.map(|saved| (saved.x, saved.y)),
            docked: saved.is_some_and(|saved| saved.docked),
            frame: None,
            ready: false,
            hidden: false,
            shown: false,
            dragging: false,
            turns: 0,
        })
    }

    fn set_content(&mut self, content: Option<Content>) {
        let some = content.is_some();
        if !some {
            self.ready = false;
        }
        if self.state.set_content(content) {
            self.open_changed();
        }
        watch::ACTIVE.store(some, Relaxed);
        self.show_or_hide();
        emit(&self.window, CONTENT_EVENT, &self.state.content);
    }

    fn hover(&mut self, inside: bool) {
        if !self.dragging && self.state.set_open(inside) {
            self.open_changed();
        }
    }

    /// 展开或收起以后。展开：先露出整个岛，再让页面播动画；收起：先让页面播动画，放完再只露出胶囊。
    fn open_changed(&mut self) {
        self.turns += 1;
        if self.state.open {
            self.set_region();
        } else {
            let turn = self.turns;
            let app = self.window.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(CLOSING).await;
                on_main_thread(&app, move || {
                    with_island(|island| {
                        // 这期间又展开了：不缩
                        if island.turns == turn {
                            island.set_region();
                        }
                    });
                });
            });
        }
        emit(&self.window, OPEN_EVENT, self.state.open);
    }
}

/// Runs `f` on the island. Does nothing when there is no island.
fn with_island<R>(f: impl FnOnce(&mut Island) -> R) -> Option<R> {
    ISLAND.with(|cell| {
        let Ok(mut island) = cell.try_borrow_mut() else {
            log::warn!("island: already in use");
            return None;
        };
        island.as_mut().map(f)
    })
}

fn on_main_thread(app: &AppHandle, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = app.run_on_main_thread(f) {
        log::warn!("island: cannot run on the main thread: {e}");
    }
}
