//! 平台特定的运行时权限。
//!
//! 当前只关心一件事：macOS 上的 Screen Recording 权限。没有它 xcap 拿不到其它
//! 进程的窗口标题（[CGWindowListCopyWindowInfo] 静默降级返回空 kCGWindowName），
//! 直接破坏「焦点窗口采集」的核心功能。
//!
//! API 形状刻意做成跨平台：Windows 直接 no-op，macOS 调 CG 系统接口。

#[cfg(target_os = "macos")]
mod macos_impl;
#[cfg(target_os = "windows")]
mod windows_impl;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod stub_impl {
    /// 其它 unix 平台 stub：默认 Granted，调用方按已授权处理。
    pub fn ensure_screen_recording() -> super::ScreenRecordingState {
        super::ScreenRecordingState::Granted
    }
}

#[cfg(target_os = "macos")]
use macos_impl as imp;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use stub_impl as imp;
#[cfg(target_os = "windows")]
use windows_impl as imp;

/// macOS Screen Recording 权限的当前状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenRecordingState {
    /// 已授权，xcap / 截屏可以工作
    Granted,
    /// 没拿到 —— 可能是从未请求过（弹框中 / 用户没决定），可能是用户显式拒绝过。
    /// CG API 没法区分这两种状态；上层只关心「能不能工作」，不区分原因。
    /// `#[allow]` 因为只在 macOS 编译目标下被 macos_impl 构造，Windows / Linux
    /// build 时 macos_impl 不参与编译 → 编译器看不到 producer → dead_code 警告。
    #[allow(dead_code)]
    NotGranted,
}

/// 启动时调用一次：preflight + 必要时显式弹框请求。
/// 弹框是同步阻塞的（macOS 在用户决定前不返回）。
///
/// 返回值 = 当前最终的状态。
pub fn ensure_screen_recording() -> ScreenRecordingState {
    imp::ensure_screen_recording()
}
