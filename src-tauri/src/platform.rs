//! 平台特定的系统集成。

/// 在系统文件管理器中打开指定目录。
pub fn open_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(path).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
    }
    Ok(())
}

/// 返回当前操作系统标识符（同 `std::env::consts::OS`）。
pub fn local_os_id() -> &'static str {
    std::env::consts::OS
}

/// 在创建主窗口后立即调用：应用平台特定的窗口微调。
///
/// Windows 11 22H2+：通过 DWM 把窗口处理成圆角，阴影自动跟着圆角走，
/// 避免 transparent + decorations:false 的矩形阴影和 CSS 圆角卡片产生四角 halo。
/// Windows 10 上调用失败（HRESULT != S_OK）但不会崩，静默忽略。
/// 其它平台：no-op。
/// 应用 Windows 系统的窗口外观调整（DWM 圆角阴影）。
#[cfg(target_os = "windows")]
pub fn apply_window_tweaks(window: &tauri::WebviewWindow) {
    use winapi::shared::windef::HWND;
    use winapi::um::dwmapi::DwmSetWindowAttribute;

    // 来自 dwmapi.h（winapi 0.3 还未加这两个常量，手动定义；值是稳定 ABI）：
    //   DWMWA_WINDOW_CORNER_PREFERENCE = 33
    //   DWMWCP_ROUND                   = 2
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;

    match window.hwnd() {
        Ok(hwnd) => {
            let pref: u32 = DWMWCP_ROUND;
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd.0 as HWND,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &pref as *const u32 as *const _,
                    std::mem::size_of::<u32>() as u32,
                );
            }
        }
        Err(e) => log::warn!("拿主窗口 HWND 失败: {e}"),
    }
}

/// 非 Windows 平台：no-op（macOS 走系统默认外观，Linux 没有特殊需求）。
#[cfg(not(target_os = "windows"))]
pub fn apply_window_tweaks(_window: &tauri::WebviewWindow) {}

/// macOS：窗口收进托盘时把 Dock 图标一并收起，重新显示时恢复。
///
/// 实现是切 NSApplication activation policy：`Regular`（正常 app，Dock + Cmd-Tab
/// 都在）↔ `Accessory`（menubar-only，Dock / Cmd-Tab 都不出现）。不切的话
/// 点 X 后窗口虽然 hide 了，Dock 图标还杵着——用户感知是"最小化到 Dock"
/// 而不是"收进右上角托盘"。
///
/// 顺序要求：show 前先切 Regular（Accessory 下 set_focus 抢不到前台），
/// hide 后再切 Accessory（反过来会先闪一下 Dock 图标消失再收窗口）。
#[cfg(target_os = "macos")]
pub fn set_dock_icon_visible(app: &tauri::AppHandle, visible: bool) {
    use tauri::ActivationPolicy;
    let policy = if visible {
        ActivationPolicy::Regular
    } else {
        ActivationPolicy::Accessory
    };
    if let Err(e) = app.set_activation_policy(policy) {
        log::warn!("切换 Dock 图标可见性失败 (visible={visible}): {e}");
    }
}

/// 非 macOS 平台：no-op（只有 macOS 有 Dock / activation policy 概念）。
#[cfg(not(target_os = "macos"))]
pub fn set_dock_icon_visible(_app: &tauri::AppHandle, _visible: bool) {}

/// Tauri 的 `App::run()` 回调。按平台分发系统级事件：
///
/// - macOS：
///   - `ExitRequested`：拦截 Cmd+Q / 关闭最后一个窗口触发的隐式退出，让 app 留在 Dock。
///     `code=Some(_)` 是程序显式 `app.exit()`（托盘"退出"），放行。
///   - `Reopen`：所有窗口都隐藏后点 Dock 图标，手动 show + focus 主窗口。
///     不处理这个事件，用户会觉得 app "卡死"——点 Dock 没反应。
/// - Windows：no-op。点关闭按钮的窗口隐藏逻辑已在 `WindowEvent::CloseRequested`
///   里处理，且 Windows 没有 Dock / Reopen 概念。
#[cfg(target_os = "macos")]
pub fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    use tauri::Manager;
    match event {
        // code: None 表示用户触发的退出（关窗 / Cmd+Q）；非 None 是 app.exit(code)，
        // 后者是程序主动退出，不阻拦。配合 MINIMIZE_TO_TRAY 才把窗关变成最小化到托盘。
        tauri::RunEvent::ExitRequested {
            api, code: None, ..
        } if crate::MINIMIZE_TO_TRAY.load(std::sync::atomic::Ordering::Relaxed) => {
            api.prevent_exit();
        }
        // 用户从 Dock 重新点 app icon（macOS Reopen 事件）：所有窗口都隐藏时把主窗调出来。
        // 正常情况下收进托盘时已切 Accessory、Dock 无图标不会触发 Reopen；这里的
        // set_dock_icon_visible(true) 是防御——万一 policy 切换失败留下了 Dock 图标，
        // 点它也能完整恢复（图标 + 窗口）而不是只弹窗口。
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => {
            if let Some(w) = app.get_webview_window("main") {
                set_dock_icon_visible(app, true);
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        _ => {}
    }
}

/// 非 macOS 平台：no-op（Reopen 仅 macOS Dock 概念）。
#[cfg(not(target_os = "macos"))]
pub fn handle_run_event(_app: &tauri::AppHandle, _event: tauri::RunEvent) {}

/// 用户最后一次鼠标 / 键盘事件距今的秒数。返回 0 = 当前活跃，大数值 = 挂机。
///
/// 用途：capture 模块每次 tick 看这个值，超过用户配置的阈值就 seal 当前会话，
/// 避免离开电脑后仍在累计"使用时长"。
///
/// - macOS：`CGEventSourceSecondsSinceLastEventType` (CoreGraphics)。
///   合并所有输入源（trackpad / 蓝牙鼠标 / 外接键盘），不分前台 / 后台 app。
/// - Windows：`GetLastInputInfo` + `GetTickCount` 算 ms 差。
///   `wrapping_sub` 处理 49 天溢出（GetTickCount u32 ms 大约 49.7 天回绕）。
/// - 其它平台：返回 0（当作永远活跃，不影响功能）。
#[cfg(target_os = "macos")]
pub fn idle_secs() -> u64 {
    use std::os::raw::c_int;
    // kCGEventSourceStateCombinedSessionState = 1
    const COMBINED_SESSION_STATE: c_int = 1;
    // kCGAnyInputEventType = 0xFFFFFFFF —— 任意键鼠 / 触控板事件
    const ANY_INPUT_EVENT: u32 = !0u32;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: c_int, event_type: u32) -> f64;
    }
    // SAFETY: CoreGraphics 公开 C API（CoreGraphics.framework），任意线程可调；
    // 返 f64 秒数，无指针 / 引用计数。无效输入返 -1.0 / NaN，下面 finite + 非负检查兜底。
    let s =
        unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT) };
    if s.is_finite() && s >= 0.0 {
        s as u64
    } else {
        0
    }
}

/// Windows 实现：用 `GetLastInputInfo` 拿距离上次鼠键输入的毫秒数。
#[cfg(target_os = "windows")]
pub fn idle_secs() -> u64 {
    use winapi::um::winuser::{GetLastInputInfo, LASTINPUTINFO};
    extern "system" {
        fn GetTickCount() -> u32;
    }
    // SAFETY: Win32 公开 API；`LASTINPUTINFO` 是栈上 POD，cbSize 已正确设置；
    // `GetTickCount` 无参数。失败时（返 0）已 early return。
    unsafe {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut info) == 0 {
            return 0;
        }
        let now = GetTickCount();
        // GetTickCount 是 u32 ms，~49.7 天溢出回绕。wrapping_sub 算出的 delta
        // 在溢出场景仍是正确的"已过 ms 数"。
        (now.wrapping_sub(info.dwTime) / 1000) as u64
    }
}

/// Linux / 其它 Unix 平台：暂不实现，统一返 0（用户视作永远活跃）。
///
/// 真正实现需要分 X11 / Wayland 两条路：
/// - X11：`XScreenSaverQueryInfo`（libXss），一个 API 搞定
/// - Wayland：`ext-idle-notify-v1` Wayland 协议（需要 portal / compositor 支持）
///
/// 当前留 stub 是产品决策——Linux 用户量小、跨发行版桌面差异大；返 0 不会触发挂机
/// 检测，但**不会破坏其它任何功能**（capture interval / privacy filter 等都正常）。
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn idle_secs() -> u64 {
    0
}
