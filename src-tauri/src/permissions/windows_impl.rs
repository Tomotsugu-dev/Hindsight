//! Windows 不需要 Screen Recording 权限 —— Win32 GDI / DWM 直接给所有窗口的 owner pid + title。
//! 留这个 stub 让上层 [super::ensure_screen_recording] 可以无条件调。

use super::ScreenRecordingState;

/// Windows 实现：直接返回 Granted（系统无 Screen Recording 权限模型）。
pub fn ensure_screen_recording() -> ScreenRecordingState {
    ScreenRecordingState::Granted
}
