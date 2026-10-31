//! 拖动：按住胶囊以后在后台线程上跟着光标挪窗口，松手时胶囊中心落在任务栏上就吸附，再存下位置。
//! 页面按鼠标移动的距离区分点击和拖动。

use std::ptr::null_mut;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use winapi::shared::windef::{HWND, POINT};
use winapi::um::winuser::{
    GetAsyncKeyState, GetCursorPos, GetSystemMetrics, SetWindowPos, SM_SWAPBUTTON, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, VK_LBUTTON, VK_RBUTTON,
};

use super::geometry::{self, Rect};
use super::saved::{self, Saved};
use super::screen::{taskbars, window_rect};
use super::{on_main_thread, with_island, Island};
use crate::island::{emit, OPEN_EVENT};

/// 拖动时多久跟一次光标
const DRAG_STEP: Duration = Duration::from_millis(8);
/// 按住以后光标移动超过这么多逻辑像素才算拖动，和页面一样
const DRAG_SLOP: f64 = 4.0;
/// 吸附着时往外拖超过这么多逻辑像素才离开任务栏
const UNDOCK_PULL: f64 = 40.0;

impl Island {
    pub(super) fn drag(&mut self) {
        if self.dragging || self.frame.is_none() {
            return;
        }
        self.dragging = true;
        let scale = self.frame.as_ref().map_or(1.0, |(_, scale)| *scale);
        let job = Drag {
            window: self.hwnd as isize,
            from: cursor(),
            origin: window_rect(self.hwnd),
            docked: self.docked,
            slop: DRAG_SLOP * scale,
            pull: UNDOCK_PULL * scale,
            button: primary_button(),
        };
        let app = self.window.app_handle().clone();
        thread::spawn(move || {
            let moved = job.follow(&app);
            on_main_thread(&app, move || {
                with_island(|island| island.dropped(moved));
            });
        });
    }

    /// 真的拖起来了：收起来，只剩胶囊。
    fn collapse_for_drag(&mut self) {
        if self.state.set_open(false) {
            self.turns += 1;
            emit(&self.window, OPEN_EVENT, false);
        }
        self.set_region();
    }

    /// 松手了。拖动过：胶囊中心落在任务栏上就吸附，再存下位置。
    fn dropped(&mut self, moved: bool) {
        self.dragging = false;
        if !moved {
            return;
        }
        let Some((frame, _)) = self.frame.clone() else {
            return;
        };
        // 窗口挪到哪，胶囊就跟着到哪
        let now = window_rect(self.hwnd);
        let capsule = Rect {
            x: now.x + frame.capsule.x - frame.window.x.round(),
            y: now.y + frame.capsule.y - frame.window.y.round(),
            ..frame.capsule
        };
        let bars: Vec<Rect> = taskbars().into_iter().map(|bar| bar.rect).collect();
        let snapped = geometry::snap(capsule, &bars);
        let at = snapped.unwrap_or((capsule.x, capsule.y));
        self.docked = snapped.is_some();
        self.at = Some(at);
        self.relayout();
        saved::save(Saved {
            x: at.0,
            y: at.1,
            docked: self.docked,
        });
    }
}

/// 按住岛以后在后台线程上跟着光标挪窗口。
struct Drag {
    /// 岛的窗口
    window: isize,
    /// 按下时光标在哪
    from: POINT,
    /// 按下时窗口在哪
    origin: Rect,
    docked: bool,
    /// 移动超过这么多物理像素才算拖动
    slop: f64,
    /// 吸附着时往外拖超过这么多物理像素才离开任务栏
    pull: f64,
    /// 主按钮的虚拟键
    button: i32,
}

impl Drag {
    /// 跟着光标挪窗口，直到松开主按钮。返回拖动了没有。
    fn follow(&self, app: &AppHandle) -> bool {
        let hwnd = self.window as HWND;
        let mut moved = false;
        let mut docked = self.docked;
        loop {
            thread::sleep(DRAG_STEP);
            let at = cursor();
            let (dx, dy) = (f64::from(at.x - self.from.x), f64::from(at.y - self.from.y));
            if !moved && (dx.abs() > self.slop || dy.abs() > self.slop) {
                moved = true;
                on_main_thread(app, || {
                    with_island(Island::collapse_for_drag);
                });
            }
            if moved {
                // 吸附着：只能沿任务栏左右移，往外拖够远才离开
                if docked && dy.abs() > self.pull {
                    docked = false;
                }
                let y = if docked {
                    self.origin.y
                } else {
                    self.origin.y + dy
                };
                // SAFETY: 窗口在主线程上，这里只挪它；主线程不等这个线程，不会卡住
                unsafe {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        (self.origin.x + dx) as i32,
                        y as i32,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
            }
            // SAFETY: 只读按键状态
            if (unsafe { GetAsyncKeyState(self.button) } as u16 & 0x8000) == 0 {
                return moved;
            }
        }
    }
}

fn cursor() -> POINT {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: point 是本地变量
    unsafe { GetCursorPos(&mut point) };
    point
}

/// 主按钮的虚拟键：左右键对调了（左手习惯）就是右键。GetAsyncKeyState 看的是物理按键。
fn primary_button() -> i32 {
    if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        VK_RBUTTON
    } else {
        VK_LBUTTON
    }
}
