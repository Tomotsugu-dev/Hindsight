//! 岛的窗口：摆在哪、露出哪一块、显示还是藏起来。

use std::ptr::null_mut;

use winapi::shared::minwindef::TRUE;
use winapi::um::wingdi::{CreateRectRgn, DeleteObject};
use winapi::um::winuser::{
    GetWindowLongPtrW, IsWindowVisible, SetWindowLongPtrW, SetWindowPos, SetWindowRgn, ShowWindow,
    GWL_EXSTYLE, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW,
};

use super::geometry::{self, Rect, CAPSULE_HEIGHT};
use super::screen::{primary_screen, screen_at};
use super::Island;
use crate::island::{emit, LAYOUT_EVENT};

impl Island {
    /// 按现在的位置、内容和那块屏的缩放比例摆窗口，设好窗口区域。
    pub(super) fn place(&mut self) {
        let (left, right) = self.state.widths;
        let width = geometry::capsule_width(left, right);
        let at = self.at.unwrap_or_else(|| {
            let primary = primary_screen();
            geometry::default_at(primary.work, primary.scale, width)
        });
        let screen = screen_at(at);
        let capsule = Rect {
            x: at.0,
            y: at.1,
            width: width * screen.scale,
            height: CAPSULE_HEIGHT * screen.scale,
        };
        let frame = geometry::frame(
            at,
            screen.rect,
            geometry::opens_up(capsule, screen.rect),
            screen.scale,
            self.state.widths,
            &self.state.panel.unwrap_or_default(),
        );
        let w = frame.window;
        // SAFETY: hwnd 是岛的窗口，岛在它就在
        unsafe {
            SetWindowPos(
                self.hwnd,
                null_mut(),
                w.x.round() as i32,
                w.y.round() as i32,
                w.width.round() as i32,
                w.height.round() as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        self.frame = Some((frame, screen.scale));
        self.set_region();
    }

    /// 重新摆，再告诉页面。
    pub(super) fn relayout(&mut self) {
        self.place();
        if let Some((frame, _)) = &self.frame {
            emit(&self.window, LAYOUT_EVENT, frame.layout);
        }
    }

    /// 窗口区域：展开时露出整个岛，收起时只露出胶囊。
    pub(super) fn set_region(&self) {
        let Some((frame, _)) = &self.frame else {
            return;
        };
        let r = if self.state.open {
            frame.open
        } else {
            frame.closed
        };
        // SAFETY: 区域交给窗口以后归系统管，没交出去就自己删掉
        unsafe {
            let region = CreateRectRgn(r.x as i32, r.y as i32, r.right() as i32, r.bottom() as i32);
            if SetWindowRgn(self.hwnd, region, TRUE) == 0 {
                DeleteObject(region.cast());
            }
        }
    }

    /// 显示或者藏起窗口：有内容、页面放好了、没有因为全屏之类藏着，才显示。
    pub(super) fn show_or_hide(&mut self) {
        let show = self.ready && self.state.content.is_some() && !self.hidden;
        // SAFETY: hwnd 是岛的窗口，岛在它就在
        let visible = unsafe { IsWindowVisible(self.hwnd) } != 0;
        if show == visible {
            return;
        }
        if !show {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            return;
        }
        if self.shown {
            unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
            return;
        }
        // 第一次用 tao 显示（建窗口时 focused(false)，tao 用的是不激活的显示方式），试验窗口就是
        // 这样验的。tao 显示时会按它记的样式重写扩展样式，还加上 WS_EX_APPWINDOW（会进 Alt+Tab），
        // 所以显示以后再改：加 WS_EX_TOOLWINDOW、去掉 WS_EX_APPWINDOW，藏一下再显示让系统认新样式。
        // 之后显示、隐藏都只用原生的 ShowWindow，不再经过 tao，免得样式又被改回去
        self.shown = true;
        if let Err(e) = self.window.show() {
            log::warn!("island: cannot show the window: {e}");
            return;
        }
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            ShowWindow(self.hwnd, SW_HIDE);
            SetWindowLongPtrW(
                self.hwnd,
                GWL_EXSTYLE,
                (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize),
            );
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }
}
