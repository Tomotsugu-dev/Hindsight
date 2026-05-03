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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenRecordingState {
    /// 已授权，xcap / 截屏可以工作
    Granted,
    /// 没拿到 —— 可能是从未请求过（弹框中 / 用户没决定），可能是用户显式拒绝过。
    /// CG API 没法区分这两种状态；上层只关心「能不能工作」，不区分原因。
    NotGranted,
}

/// 启动时调用一次：preflight + 必要时显式弹框请求。
/// 弹框是同步阻塞的（macOS 在用户决定前不返回）。
///
/// 返回值 = 当前最终的状态。
pub fn ensure_screen_recording() -> ScreenRecordingState {
    #[cfg(target_os = "macos")]
    {
        macos_impl::ensure_screen_recording()
    }
    #[cfg(target_os = "windows")]
    {
        windows_impl::ensure_screen_recording()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        ScreenRecordingState::Granted
    }
}
