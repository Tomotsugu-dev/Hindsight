//! The island window on macOS. The window is always the size of the open island at its largest. A
//! Core Animation mask shows only the closed shapes or the open panel, so the window never resizes
//! and the page never re-lays out while it animates. A tracking area tells when the mouse goes over
//! the island and leaves it, also while another app is active. 窗口是不激活的 NSPanel：点岛不会
//! 让 Hindsight 变成前台 app，用户正在用的 app 还在前台。
//!
//! 这个文件管岛上的内容、展开收起和摆窗口，别的一个功能一个文件：
//! - `geometry.rs`：形状在哪，只算不碰窗口
//! - `panel.rs`：不激活的 NSPanel，在哪一层、跨桌面怎么显示
//! - `mask.rs`：遮罩和它的动画
//! - `tracking.rs`：鼠标移入移出、屏幕变化
//! - `layout.rs`：主屏、给页面的布局、几种坐标之间的换算
//!
//! AppKit objects may only be used on the main thread, so the island lives there.
//!
//! When the content changes:
//! 1. this sends it to the page;
//! 2. the page draws it, measures the closed content and the open panel, and calls [`measured`],
//!    which returns where each side goes;
//! 3. the page puts the content there and calls [`ready`]. Only then does the window show, so it
//!    never shows content that is not in place.

mod geometry;
mod layout;
mod mask;
mod panel;
mod tracking;

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSTrackingArea, NSView, NSWindow};
use objc2_quartz_core::{CALayer, CATransaction};
use tauri::{AppHandle, WebviewWindow};

use super::content::Content;
use super::{emit, new_window, Layout, PanelSize, State, CONTENT_EVENT, LAYOUT_EVENT, OPEN_EVENT};
use geometry::Screen;
use layout::{local, main_screen, ns_rect};
use tracking::Observer;

struct Island {
    window: WebviewWindow,
    ns_window: Retained<NSWindow>,
    view: Retained<NSView>,
    mask: Retained<CALayer>,
    /// Two black layers in the mask. Open, both fill the panel. Closed, each covers one shape, or
    /// both cover the only shape, so a side can come and go without adding or removing a layer
    shapes: [Retained<CALayer>; 2],
    observer: Retained<Observer>,
    area: Retained<NSTrackingArea>,
    screen: Screen,
    state: State,
}

thread_local! {
    static ISLAND: RefCell<Option<Island>> = const { RefCell::new(None) };
}

/// Shows `content` on the island in place of what is there. Opens the window the first time.
pub fn show(app: &AppHandle, content: Content) {
    let handle = app.clone();
    on_main_thread(app, move |mtm| {
        ISLAND.with(|cell| {
            let Ok(mut island) = cell.try_borrow_mut() else {
                log::warn!("island: already in use");
                return;
            };
            if island.is_none() {
                match Island::new(&handle, mtm) {
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
    on_main_thread(app, |_| {
        with_island(|island| island.set_content(None));
    });
}

/// Returns what is on the island now, for the page when it loads. 要在主线程上调用。
pub fn content() -> Option<Content> {
    main_thread()?;
    with_island(|island| island.state.content.clone()).flatten()
}

/// Takes the widths of the closed content and the size of the open content measured by the page,
/// and returns where each side goes. `None` when there is no island. 要在主线程上调用。
pub fn measured(left: Option<f64>, right: Option<f64>, panel: Option<PanelSize>) -> Option<Layout> {
    main_thread()?;
    with_island(|island| {
        island.state.measured(left, right, panel);
        // A side that grows or shrinks in view moves smoothly
        island.draw(island.ns_window.isVisible());
        island.track();
        island.layout()
    })
}

/// Shows the window: the page has put the content in place. Does nothing when nothing is on the
/// island. 要在主线程上调用。
pub fn ready() {
    if main_thread().is_none() {
        return;
    }
    with_island(|island| {
        if island.state.content.is_some() {
            island.ns_window.orderFrontRegardless();
        }
    });
}

/// 什么都不做：鼠标移入移出由追踪区域报告，页面不用报。
pub fn hover(_inside: bool) {}

/// 什么都不做：菜单栏上的岛不能拖。
pub fn drag() {}

impl Island {
    fn new(app: &AppHandle, mtm: MainThreadMarker) -> Result<Self, String> {
        let screen = main_screen(mtm).ok_or("there is no screen")?;
        // A click on the island works while another app is active
        let window = new_window(app, |builder| builder.accept_first_mouse(true))?;
        Self::set_up(window.clone(), screen, mtm).inspect_err(|_| {
            // Or the next try would fail too: the label is taken
            let _ = window.destroy();
        })
    }

    /// Makes `window` an island: above the menu bar, on every desktop, showing only what the mask
    /// covers.
    fn set_up(
        window: WebviewWindow,
        screen: Screen,
        mtm: MainThreadMarker,
    ) -> Result<Self, String> {
        let ns_window = window.ns_window().map_err(|e| e.to_string())?;
        // SAFETY: Tauri gives the window's NSWindow, which lives as long as the window
        let ns_window = unsafe { Retained::retain(ns_window.cast::<NSWindow>()) }
            .ok_or("the window has no NSWindow")?;
        panel::set_up(&ns_window);
        let view = ns_window.contentView().ok_or("the window has no view")?;
        let (mask, shapes) = mask::new(&view)?;
        let (observer, area) = tracking::observe(&view, mtm);
        let mut island = Self {
            window,
            ns_window,
            view,
            mask,
            shapes,
            observer,
            area,
            screen,
            state: State::default(),
        };
        island.place();
        Ok(island)
    }

    fn set_content(&mut self, content: Option<Content>) {
        if content.is_none() {
            self.ns_window.orderOut(None);
        }
        if self.state.set_content(content) {
            self.open_changed();
        }
        emit(&self.window, CONTENT_EVENT, &self.state.content);
    }

    fn set_open(&mut self, open: bool) {
        if self.state.set_open(open) {
            self.open_changed();
        }
    }

    /// 展开或收起以后：遮罩和追踪区域跟着变，再告诉页面。
    fn open_changed(&mut self) {
        self.draw(true);
        self.track();
        emit(&self.window, OPEN_EVENT, self.state.open);
    }

    fn screens_changed(&mut self, mtm: MainThreadMarker) {
        // With no screen, for example while the screens sleep, it stays where it was
        let Some(screen) = main_screen(mtm) else {
            return;
        };
        self.screen = screen;
        self.place();
        emit(&self.window, LAYOUT_EVENT, self.layout());
    }

    /// Puts the window and the mask on the main screen as it is now, and sets whether the island
    /// shows over full-screen apps on this screen.
    fn place(&mut self) {
        let window = geometry::window(&self.screen);
        self.ns_window.setFrame_display(ns_rect(window), true);
        self.ns_window
            .setCollectionBehavior(panel::collection_behavior(self.screen.notch.is_some()));
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.mask.setFrame(local(window, window));
        CATransaction::commit();
        self.draw(false);
        self.track();
    }
}

/// Runs `f` on the island. Does nothing when there is no island.
fn with_island<R>(f: impl FnOnce(&mut Island) -> R) -> Option<R> {
    ISLAND.with(|cell| {
        // AppKit sends mouse and screen messages from the event loop, never while this runs.
        // Checked anyway: a panic would end the app
        let Ok(mut island) = cell.try_borrow_mut() else {
            log::warn!("island: already in use");
            return None;
        };
        island.as_mut().map(f)
    })
}

/// 不在主线程上时返回 `None`，并记日志：岛只在主线程上，别的线程看到的岛总是空的。
fn main_thread() -> Option<MainThreadMarker> {
    let mtm = MainThreadMarker::new();
    if mtm.is_none() {
        log::warn!("island: not on the main thread");
    }
    mtm
}

fn on_main_thread(app: &AppHandle, f: impl FnOnce(MainThreadMarker) + Send + 'static) {
    let result = app.run_on_main_thread(move || match MainThreadMarker::new() {
        Some(mtm) => f(mtm),
        None => log::warn!("island: not on the main thread"),
    });
    if let Err(e) = result {
        log::warn!("island: cannot run on the main thread: {e}");
    }
}
