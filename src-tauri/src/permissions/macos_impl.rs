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

/// 标记文件：表明已经向用户弹过一次 CGRequestScreenCaptureAccess 系统对话框。
/// 落在 `<data_root>/.screen_recording_requested`，跨升级保留，跨用户隔离（数据
/// 目录已经是 per-user 的）。删除该文件即可让下次启动重新弹框。
const REQUESTED_MARKER_FILE: &str = ".screen_recording_requested";

/// macOS 实现：先 `CGPreflightScreenCaptureAccess` preflight 查权限。
///
/// preflight=false 时分两种处理：
/// - 标记文件**不存在**（第一次启动）→ 调 `CGRequestScreenCaptureAccess` 弹一次
///   系统对话框，写下标记
/// - 标记文件**已存在** → 跳过弹框，返回 `NotGranted` 让 UI 提示用户去系统设置授权
///
/// 这样规避了 macOS 15 (Sequoia) 上 `CGPreflightScreenCaptureAccess` 在已授权
/// 状态下偶尔返回 false 的诡异行为——之前那个 bug 表现是用户已授权了系统弹框还
/// 每次启动都跳出来。
pub fn ensure_screen_recording() -> ScreenRecordingState {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return ScreenRecordingState::Granted;
    }

    let marker = crate::bootstrap::data_root().join(REQUESTED_MARKER_FILE);
    if marker.exists() {
        log::warn!(
            "Screen Recording preflight=false 但已请求过；跳过系统弹框（去系统设置 → 隐私与安全性 手动授权）"
        );
        return ScreenRecordingState::NotGranted;
    }

    // 第一次启动：弹框，同步阻塞直到用户决定
    let granted = unsafe { CGRequestScreenCaptureAccess() };
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&marker, b"1");

    if granted {
        ScreenRecordingState::Granted
    } else {
        // CGRequestScreenCaptureAccess 在用户首次"允许"时通常返回 true，但实际生效
        // 还要等下一次进程启动 —— TCC 在进程启动时缓存权限。所以即便此处返回 false，
        // 下次启动 preflight 会变 true。
        ScreenRecordingState::NotGranted
    }
}
