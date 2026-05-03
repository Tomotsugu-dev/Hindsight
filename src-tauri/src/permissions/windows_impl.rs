//! Windows 不需要 Screen Recording 权限 —— Win32 GDI / DWM 直接给所有窗口的 owner pid + title。
//! 留这个 stub 让上层 [super::ensure_screen_recording] 可以无条件调。

use super::ScreenRecordingState;

pub fn ensure_screen_recording() -> ScreenRecordingState {
    ScreenRecordingState::Granted
}
