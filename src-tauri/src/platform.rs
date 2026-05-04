//! 平台特定的窗口微调。当前只有 Windows 上的 DWM 圆角。
//!
//! 为什么需要：transparent + decorations:false 的 Tauri 窗口本身是矩形的，
//! 圆角是 CSS `border-radius` 画的。DWM 阴影按"窗口形状"画 → 矩形阴影
//! 边缘 vs 圆角卡片轮廓 = 四角处有视觉不一致。
//!
//! Windows 11 22H2+ 提供 `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE)`，
//! 让 OS 把窗口本身处理成圆角，DWM 阴影自动跟着圆角走。Windows 10 上调用会
//! 失败（HRESULT != S_OK）但不会崩，静默忽略即可。

#[cfg(target_os = "windows")]
pub fn enable_win11_rounded_corners(hwnd_value: isize) {
    use winapi::shared::windef::HWND;
    use winapi::um::dwmapi::DwmSetWindowAttribute;

    // 来自 dwmapi.h：
    //   DWMWA_WINDOW_CORNER_PREFERENCE = 33
    //   DWMWCP_ROUND                   = 2  (Win11 默认窗口圆角，约 8 px)
    // winapi 0.3 还没加这两个常量，手动定义即可——值是稳定的 ABI。
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;

    let pref: u32 = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd_value as HWND,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const u32 as *const _,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub fn enable_win11_rounded_corners(_hwnd_value: isize) {
    // no-op on non-Windows
}
