//! macOS Screen Recording 权限请求（CoreGraphics 私有友好 API，macOS 11+ 公开）。
//!
//! - `CGPreflightScreenCaptureAccess()` 仅查询，不弹框
//! - `CGRequestScreenCaptureAccess()` 没权限就弹系统对话框、把当前 app 加进
//!   System Settings → Privacy & Security → Screen Recording 列表，同步等用户决定
//!
//! 这俩 API 都来自 CoreGraphics.framework。Tauri 已经间接连了 CG，不用额外加 link 配置。

use super::ScreenRecordingState;

extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// macOS 实现：先 `CGPreflightScreenCaptureAccess` preflight，没拿到再调
/// `CGRequestScreenCaptureAccess` 触发系统弹框（同步阻塞直到用户决定）。
/// `granted` 后续 spawn 的 xcap 才能拿其它进程的窗口标题。
pub fn ensure_screen_recording() -> ScreenRecordingState {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return ScreenRecordingState::Granted;
    }
    // preflight=false 包含两种状态（Denied / NotDetermined），CG API 没法区分。
    // 这里直接调 Request：NotDetermined 会弹框；Denied 会无声打开系统设置。
    let granted = unsafe { CGRequestScreenCaptureAccess() };
    if granted {
        ScreenRecordingState::Granted
    } else {
        // CGRequestScreenCaptureAccess 在用户首次"允许"时通常返回 true，但实际生效
        // 还要等下一次进程启动 —— TCC 在进程启动时缓存权限。所以即便此处返回 false，
        // 下次启动 preflight 会变 true。
        ScreenRecordingState::NotGranted
    }
}
