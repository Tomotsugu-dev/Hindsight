//! 屏幕和任务栏在哪，物理像素；还有几个读窗口的小函数。

use std::ptr::{null, null_mut};

use winapi::shared::windef::{HMONITOR, HWND, POINT, RECT};
use winapi::um::shellscalingapi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use winapi::um::winuser::{
    FindWindowExW, GetClassNameW, GetMonitorInfoW, GetWindowRect, MonitorFromPoint,
    MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
    MONITOR_DEFAULTTOPRIMARY,
};

use super::geometry::Rect;

/// 一块屏：物理像素，和它的缩放比例。
pub(super) struct Screen {
    pub(super) handle: HMONITOR,
    pub(super) rect: Rect,
    /// 去掉任务栏的部分
    pub(super) work: Rect,
    pub(super) scale: f64,
}

fn screen_of(handle: HMONITOR) -> Screen {
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    let (mut dpi, mut unused) = (96, 96);
    // SAFETY: handle 是系统刚给的；info 和 dpi 是本地变量
    unsafe {
        GetMonitorInfoW(handle, &mut info);
        GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
    }
    Screen {
        handle,
        rect: rect(info.rcMonitor),
        work: rect(info.rcWork),
        scale: f64::from(dpi) / 96.0,
    }
}

pub(super) fn primary_screen() -> Screen {
    screen_of(unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) })
}

/// `point` 所在的屏；不在任何一块屏上时取最近的那块。
pub(super) fn screen_at(point: (f64, f64)) -> Screen {
    screen_of(unsafe { MonitorFromPoint(to_point(point), MONITOR_DEFAULTTONEAREST) })
}

pub(super) fn on_a_screen(point: (f64, f64)) -> bool {
    !unsafe { MonitorFromPoint(to_point(point), MONITOR_DEFAULTTONULL) }.is_null()
}

/// 一条任务栏，和它所在的屏。
pub(super) struct Taskbar {
    pub(super) monitor: HMONITOR,
    pub(super) rect: Rect,
}

/// 各块屏上的任务栏：主屏的 Shell_TrayWnd，别的屏的 Shell_SecondaryTrayWnd。
pub(super) fn taskbars() -> Vec<Taskbar> {
    let mut bars = Vec::new();
    for class in ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
        let class = wide(class);
        let mut after = null_mut();
        loop {
            // SAFETY: class 是以 0 结尾的 UTF-16；after 是上一次找到的窗口或者空
            let bar = unsafe { FindWindowExW(null_mut(), after, class.as_ptr(), null()) };
            if bar.is_null() {
                break;
            }
            bars.push(Taskbar {
                // 自动隐藏缩下去时大半在屏幕外，取最近的那块屏
                monitor: unsafe { MonitorFromWindow(bar, MONITOR_DEFAULTTONEAREST) },
                rect: window_rect(bar),
            });
            after = bar;
        }
    }
    bars
}

pub(super) fn window_rect(hwnd: HWND) -> Rect {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: r 是本地变量；窗口没了时读不到，r 还是 0
    unsafe { GetWindowRect(hwnd, &mut r) };
    rect(r)
}

pub(super) fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    // SAFETY: buf 是本地变量，长度如实传进去
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn rect(r: RECT) -> Rect {
    Rect {
        x: f64::from(r.left),
        y: f64::from(r.top),
        width: f64::from(r.right - r.left),
        height: f64::from(r.bottom - r.top),
    }
}

fn to_point((x, y): (f64, f64)) -> POINT {
    POINT {
        x: x as i32,
        y: y as i32,
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
