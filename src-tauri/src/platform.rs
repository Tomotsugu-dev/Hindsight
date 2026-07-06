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

// ─────────────────────────────────────────────────────────────
// 电源来源（交流 / 电池）——耗电任务的调度信号
//
// 背景：OCR 消化是持续的 CPU 推理负载，设计要求"插电常驻，拔电即退"
// （docs/design/screen-memory.md §6）。所有实现遵循 fail-open：探测失败
// 一律按"插电"处理——电源感知是省电优化，探测异常不能反过来卡死消化。
// ─────────────────────────────────────────────────────────────

/// 当前是否接通外部电源。台式机恒 true；探测失败也返 true（fail-open）。
#[cfg(target_os = "macos")]
pub fn on_ac_power() -> bool {
    // IOKit 电源约定：
    //   kIOPSTimeRemainingUnlimited = -2.0 → 接通电源
    //   kIOPSTimeRemainingUnknown   = -1.0 → 电池供电（剩余时间未算出）
    //   >= 0                              → 电池供电的剩余秒数
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSGetTimeRemainingEstimate() -> f64;
    }
    // SAFETY: IOKit 公开 C API，无参无指针，任意线程可调，返回 f64。
    let t = unsafe { IOPSGetTimeRemainingEstimate() };
    if t.is_nan() {
        return true; // 异常值按插电（fail-open）
    }
    // t > -1.5 ⇔ 明确的电池信号（-1.0 或非负剩余秒）；-2.0 → 插电。
    t <= -1.5
}

/// Windows 实现：`GetSystemPowerStatus` 的 ACLineStatus。
#[cfg(target_os = "windows")]
pub fn on_ac_power() -> bool {
    use winapi::um::winbase::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: Win32 公开 API；出参结构体是栈上 POD，由系统填充；失败返 0，
    // 已 early return 按插电处理。
    unsafe {
        let mut st: SYSTEM_POWER_STATUS = std::mem::zeroed();
        if GetSystemPowerStatus(&mut st) == 0 {
            return true;
        }
        // ACLineStatus：0=电池，1=交流，255=未知（非 0 → 按插电，fail-open）
        st.ACLineStatus != 0
    }
}

/// 其它平台：视作恒插电（不启用电源门控）。
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn on_ac_power() -> bool {
    true
}

// ─────────────────────────────────────────────────────────────
// 屏幕不可看状态（息屏 / 锁屏 / 屏保）——挂机判定的硬信号
//
// 背景：capture 的挂机判定在键鼠空闲后靠"屏幕活跃探测"豁免被动观看（看视频/
// 盯编译）。但 AIDA64 传感器面板、跳动的时钟这类"屏幕永远在变"的前台会把
// 豁免变成永动机——用户离开 9 小时仍被记成使用。息屏/锁屏/屏保是"人必然
// 不在看"的确定信号，比任何像素启发式都硬，capture tick 见到即无条件封会话。
// ─────────────────────────────────────────────────────────────

/// 屏幕当前是否处于"不可看"状态（息屏 / 锁屏 / 屏保任一命中）。
///
/// Windows：息屏与锁屏来自常驻消息窗口线程接收的系统通知
/// （`GUID_CONSOLE_DISPLAY_STATE` 电源通知 + WTS 会话锁通知），首次调用时
/// 惰性启动该线程；屏保用 `SPI_GETSCREENSAVERRUNNING` 即时轮询。
/// 通知注册失败只降级（恒返 false = 回到今天的行为），不影响其它功能。
#[cfg(target_os = "windows")]
pub fn screen_unavailable() -> bool {
    windows_screen_state::ensure_watcher();
    windows_screen_state::display_off()
        || windows_screen_state::session_locked()
        || windows_screen_state::screensaver_running()
}

/// macOS：主显示器睡眠 = 息屏（CoreGraphics 直接可查，无需通知线程）。
/// 锁屏/屏保已由 capture 侧的前台占位进程判定（loginwindow / ScreenSaverEngine）
/// 覆盖，这里不重复。
#[cfg(target_os = "macos")]
pub fn screen_unavailable() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayIsAsleep(display: u32) -> i32;
    }
    // SAFETY: CoreGraphics 公开 C API，任意线程可调，无指针传递。
    unsafe { CGDisplayIsAsleep(CGMainDisplayID()) != 0 }
}

/// 其它平台：无检测（恒可看），挂机判定回退纯键鼠信号。
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn screen_unavailable() -> bool {
    false
}

/// 前台是否是桌面本体（Windows：`Progman` / `WorkerW` 窗口类）。
///
/// 用途：焦点停在桌面时，"屏幕在变"不代表有人在看——Wallpaper Engine 等动态
/// 壁纸让桌面永远在变，挂机判定的被动观看豁免对桌面前台不适用。
#[cfg(target_os = "windows")]
pub fn is_desktop_foreground() -> bool {
    use winapi::um::winuser::{GetClassNameW, GetForegroundWindow};
    // SAFETY: Win32 公开 API；GetForegroundWindow 可能返 null（已判）；
    // GetClassNameW 写入栈上定长缓冲，返回实际长度。
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        let mut buf = [0u16; 16];
        let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len <= 0 {
            return false;
        }
        let class = String::from_utf16_lossy(&buf[..len as usize]);
        class == "Progman" || class == "WorkerW"
    }
}

/// 非 Windows：桌面前台概念不适用（macOS 桌面焦点是 Finder，正常应用语义）。
#[cfg(not(target_os = "windows"))]
pub fn is_desktop_foreground() -> bool {
    false
}

/// Windows 息屏/锁屏状态源：隐藏消息窗口线程 + 两个 AtomicBool。
#[cfg(target_os = "windows")]
mod windows_screen_state {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Once;

    use winapi::shared::guiddef::{IsEqualGUID, GUID};
    use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
    use winapi::shared::windef::HWND;
    use winapi::um::libloaderapi::GetModuleHandleW;
    use winapi::um::winuser::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
        RegisterPowerSettingNotification, SystemParametersInfoW, TranslateMessage,
        DEVICE_NOTIFY_WINDOW_HANDLE, MSG, PBT_POWERSETTINGCHANGE, POWERBROADCAST_SETTING,
        SPI_GETSCREENSAVERRUNNING, WM_POWERBROADCAST, WNDCLASSW,
    };

    /// 显示器电源状态通知的 GUID（winapi 0.3 未导出，值是稳定 ABI）：
    /// GUID_CONSOLE_DISPLAY_STATE = 6FE69556-704A-47A0-8F24-C28D936FDA47
    const GUID_CONSOLE_DISPLAY_STATE: GUID = GUID {
        Data1: 0x6fe6_9556,
        Data2: 0x704a,
        Data3: 0x47a0,
        Data4: [0x8f, 0x24, 0xc2, 0x8d, 0x93, 0x6f, 0xda, 0x47],
    };
    /// 会话变更消息与锁/解锁事件码（WinUser.h，稳定 ABI）。
    const WM_WTSSESSION_CHANGE: UINT = 0x02B1;
    const WTS_SESSION_LOCK: WPARAM = 0x7;
    const WTS_SESSION_UNLOCK: WPARAM = 0x8;
    /// WTSRegisterSessionNotification 的 dwFlags：只关心本会话。
    const NOTIFY_FOR_THIS_SESSION: u32 = 0;

    static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);
    static SESSION_LOCKED: AtomicBool = AtomicBool::new(false);
    static WATCHER: Once = Once::new();

    pub fn display_off() -> bool {
        DISPLAY_OFF.load(Ordering::Relaxed)
    }

    pub fn session_locked() -> bool {
        SESSION_LOCKED.load(Ordering::Relaxed)
    }

    /// 屏保是否正在运行（即时查询，无需通知）。
    pub fn screensaver_running() -> bool {
        let mut running: i32 = 0;
        // SAFETY: Win32 公开 API，SPI_GETSCREENSAVERRUNNING 写入一个 BOOL。
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETSCREENSAVERRUNNING,
                0,
                &mut running as *mut i32 as *mut _,
                0,
            )
        };
        ok != 0 && running != 0
    }

    /// 惰性启动通知线程（进程生命周期内常驻，无需回收）。
    pub fn ensure_watcher() {
        WATCHER.call_once(|| {
            std::thread::Builder::new()
                .name("screen-state-watcher".into())
                .spawn(run_message_window)
                .map(|_| ())
                .unwrap_or_else(|e| log::warn!("屏幕状态监听线程启动失败: {e}"));
        });
    }

    /// 消息窗口线程体：注册隐藏窗口 → 订阅显示器电源 + 会话锁通知 → 消息循环。
    /// 任何一步失败都只 log 降级（两个 Atomic 保持 false）。
    fn run_message_window() {
        // WTSRegisterSessionNotification 在 wtsapi32.dll——winapi 的 wtsapi32
        // feature 未导出此函数，手动声明（稳定导出符号）。
        #[link(name = "wtsapi32")]
        extern "system" {
            fn WTSRegisterSessionNotification(hwnd: HWND, dw_flags: u32) -> i32;
        }
        // HWND_MESSAGE = -3：message-only window 的父句柄哨兵值。
        const HWND_MESSAGE: HWND = -3isize as HWND;

        let class_name: Vec<u16> = "hindsight_screen_state\0".encode_utf16().collect();
        // SAFETY: 标准 Win32 消息窗口样板。类名/窗口名指针在调用期间有效;
        // wndproc 是本模块的 extern "system" 函数;消息循环单线程运行。
        unsafe {
            let hinstance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&wc) == 0 {
                log::warn!("屏幕状态监听: RegisterClassW 失败");
                return;
            }
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null_mut(),
            );
            if hwnd.is_null() {
                log::warn!("屏幕状态监听: CreateWindowExW 失败");
                return;
            }
            if RegisterPowerSettingNotification(
                hwnd as *mut _,
                &GUID_CONSOLE_DISPLAY_STATE,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
            .is_null()
            {
                log::warn!("屏幕状态监听: 显示器电源通知注册失败(息屏检测降级)");
            }
            if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) == 0 {
                log::warn!("屏幕状态监听: 会话锁通知注册失败(锁屏检测降级)");
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// 窗口过程：只消费两类消息,其余全部交回 DefWindowProc。
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: UINT,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_POWERBROADCAST if wparam == PBT_POWERSETTINGCHANGE as WPARAM => {
                let s = &*(lparam as *const POWERBROADCAST_SETTING);
                if IsEqualGUID(&s.PowerSetting, &GUID_CONSOLE_DISPLAY_STATE) {
                    // Data 载荷是一个 DWORD：0=息屏 1=亮屏 2=调暗(仍可看,不算息屏)
                    let state = *(s.Data.as_ptr() as *const u32);
                    DISPLAY_OFF.store(state == 0, Ordering::Relaxed);
                    log::debug!("显示器电源状态变更: {state}");
                }
                1 // TRUE
            }
            WM_WTSSESSION_CHANGE => {
                match wparam {
                    WTS_SESSION_LOCK => SESSION_LOCKED.store(true, Ordering::Relaxed),
                    WTS_SESSION_UNLOCK => SESSION_LOCKED.store(false, Ordering::Relaxed),
                    _ => {}
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
