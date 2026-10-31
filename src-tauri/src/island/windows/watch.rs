//! 盯着前台和任务栏，见 [`Island::tick`]。前台窗口换了马上查一遍，岛上有内容时每隔 [`TICK`] 也查
//! 一遍，都在主线程上查。

use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread;
use std::time::Duration;

use tauri::AppHandle;
use winapi::shared::minwindef::DWORD;
use winapi::shared::ntdef::LONG;
use winapi::shared::windef::{HMONITOR, HWINEVENTHOOK, HWND, POINT};
use winapi::um::winuser::{
    GetAncestor, GetForegroundWindow, GetWindowLongW, MonitorFromWindow, SetWinEventHook,
    SetWindowPos, WindowFromPoint, EVENT_SYSTEM_FOREGROUND, GA_ROOT, GWL_STYLE, HWND_TOPMOST,
    MONITOR_DEFAULTTONULL, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WINEVENT_OUTOFCONTEXT,
    WS_CAPTION,
};

use super::geometry::{self, Frame, Rect};
use super::screen::{class_name, on_a_screen, screen_at, taskbars, window_rect};
use super::{Island, ISLAND};

/// 多久查一次全屏、任务栏、有没有被任务栏压住
const TICK: Duration = Duration::from_millis(150);

/// 岛上有内容：只有这时后台才每隔一会儿让主线程查一遍
pub(super) static ACTIVE: AtomicBool = AtomicBool::new(false);

/// 开始盯着。要在主线程上调用：前台换了的回调在装它的线程上执行。
pub(super) fn start(app: &AppHandle) {
    // 前台换了马上查一遍：点了任务栏、开了全屏的程序
    // SAFETY: 回调是这个文件里的函数，一直都在
    unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            null_mut(),
            Some(foreground_changed),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    let app = app.clone();
    thread::spawn(move || loop {
        thread::sleep(TICK);
        if ACTIVE.load(Relaxed) && app.run_on_main_thread(tick).is_err() {
            break;
        }
    });
}

impl Island {
    /// 每隔一会儿、前台换了的时候查一遍：
    /// - 胶囊所在的屏没了（比如拔了外接屏）：回到默认位置；
    /// - 窗口被系统挪了（到了缩放比例不同的屏），或者那块屏的缩放比例改了：重新摆；
    /// - 吸附着的任务栏变高变矮了：重新居中；
    /// - 前台全屏，或者吸附着的任务栏缩下去了：藏起来；
    /// - 吸附着被任务栏压住了：提回最上面。
    fn tick(&mut self) {
        if self.state.content.is_none() || self.dragging {
            return;
        }
        let Some((frame, scale)) = self.frame.clone() else {
            return;
        };
        if self.at.is_some_and(|at| !on_a_screen(at)) {
            self.at = None;
            self.docked = false;
            self.relayout();
            return;
        }
        let screen = screen_at((frame.capsule.x, frame.capsule.y));
        let now = window_rect(self.hwnd);
        if screen.scale != scale
            || now.x != frame.window.x.round()
            || now.y != frame.window.y.round()
        {
            self.relayout();
            return;
        }
        let bar = taskbars()
            .into_iter()
            .find(|bar| bar.monitor == screen.handle);
        if self.docked {
            // 任务栏缩下去时胶囊中心不在它上面，不动
            let centered = bar
                .as_ref()
                .and_then(|bar| geometry::snap(frame.capsule, &[bar.rect]));
            if let (Some(at), Some(old)) = (centered, self.at) {
                if (at.0 - old.0).abs() >= 1.0 || (at.1 - old.1).abs() >= 1.0 {
                    self.at = Some(at);
                    self.relayout();
                    return;
                }
            }
        }
        let hidden = full_screen(screen.handle, screen.rect, self.hwnd)
            || (self.docked
                && !bar.is_some_and(|bar| geometry::taskbar_shown(bar.rect, screen.rect)));
        if hidden != self.hidden {
            self.hidden = hidden;
            self.show_or_hide();
        }
        if self.docked && !self.hidden {
            self.raise(&frame);
        }
    }

    /// 吸附着的时候，胶囊中间那一点上面不是自己（被任务栏压住了），就提回最上面。开始菜单开着时
    /// 任务栏在更高的一层，提不上去；开始菜单关了就回来了。
    fn raise(&self, frame: &Frame) {
        let (x, y) = frame.capsule.center();
        // SAFETY: 只读窗口的位置，再改岛自己的窗口
        unsafe {
            let top = GetAncestor(
                WindowFromPoint(POINT {
                    x: x as i32,
                    y: y as i32,
                }),
                GA_ROOT,
            );
            if top != self.hwnd {
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
}

/// 前台是不是全屏的程序，在 `monitor` 这块屏上：没有标题栏，盖满整块屏。
///
/// 不用 SHQueryUserNotificationState：没有程序全屏时它也会报 QUNS_BUSY（试验窗口实测）。
fn full_screen(monitor: HMONITOR, screen: Rect, island: HWND) -> bool {
    // SAFETY: 只读前台窗口的样式和位置
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() || fg == island || MonitorFromWindow(fg, MONITOR_DEFAULTTONULL) != monitor {
            return false;
        }
        // 桌面也没有标题栏、盖满整块屏
        if matches!(
            class_name(fg).as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
        let caption = (GetWindowLongW(fg, GWL_STYLE) as u32 & WS_CAPTION) == WS_CAPTION;
        geometry::covers(window_rect(fg), screen, caption)
    }
}

/// 前台窗口换了：马上查一遍，不等下一次。
unsafe extern "system" fn foreground_changed(
    _: HWINEVENTHOOK,
    _: DWORD,
    _: HWND,
    _: LONG,
    _: LONG,
    _: DWORD,
    _: DWORD,
) {
    tick();
}

/// 查一遍，见 [`Island::tick`]。岛正被别处用着就跳过：下一次再查。
fn tick() {
    ISLAND.with(|cell| {
        if let Ok(mut island) = cell.try_borrow_mut() {
            if let Some(island) = island.as_mut() {
                island.tick();
            }
        }
    });
}
