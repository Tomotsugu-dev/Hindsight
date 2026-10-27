//! 平台特定的系统集成。

use crate::error::{Error, Result};

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

/// 系统语言，对应到应用支持的六种之一：繁体圈（台湾 / 香港 / 澳门 / Hant 脚本）→ "tw"、
/// 其余中文 → "zh"、日语 → "ja"、葡萄牙语 → "pt"、西班牙语 → "es"；读不到或不在这几种里 → "en"。
pub fn system_language() -> &'static str {
    match sys_locale::get_locale() {
        Some(loc) => {
            let l = loc.to_ascii_lowercase();
            if l.starts_with("zh") {
                let hant = [
                    "zh-tw", "zh_tw", "zh-hk", "zh_hk", "zh-mo", "zh_mo", "zh-hant", "zh_hant",
                ];
                // If the locale matches any of the traditional Chinese variants, return "tw".
                if hant.iter().any(|p| l.starts_with(p)) {
                    "tw"
                } else {
                    "zh"
                }
            } else if l.starts_with("ja") {
                "ja"
            } else if l.starts_with("pt") {
                "pt"
            } else if l.starts_with("es") {
                "es"
            } else {
                "en"
            }
        }
        None => "en",
    }
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

/// 在任务栏按钮上叠 `dot`，`dot` 为 `None` 时去掉。
#[cfg(target_os = "windows")]
pub fn set_taskbar_dot(
    window: &tauri::Window,
    dot: Option<tauri::image::Image<'static>>,
) -> tauri::Result<()> {
    window.set_overlay_icon(dot)
}

/// 非 Windows 平台：no-op（只有 Windows 的任务栏按钮能叠图）。
#[cfg(not(target_os = "windows"))]
pub fn set_taskbar_dot(
    _window: &tauri::Window,
    _dot: Option<tauri::image::Image<'static>>,
) -> tauri::Result<()> {
    Ok(())
}

/// The notification sound name that plays the system alert sound. `None`: the platform has no
/// such name, and notifications stay silent.
#[cfg(target_os = "macos")]
pub const NOTIFICATION_SOUND: Option<&str> = Some("NSUserNotificationDefaultSoundName");

/// The notification sound name that plays the system alert sound.
#[cfg(target_os = "windows")]
pub const NOTIFICATION_SOUND: Option<&str> = Some("Default");

/// Other platforms: notifications stay silent.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const NOTIFICATION_SOUND: Option<&str> = None;

/// Auto start argument for Windows platform.
pub const AUTOSTART_ARG: &str = "--autostart";

/// Returns whether this process was launched as a macOS login item.
///
/// macOS login items cannot pass command-line arguments. Instead, the system
/// attaches `keyAELaunchedAsLogInItem` (`lgit`) to the initial `kAEOpenApplication` (`oapp`)
/// Apple Event when launching an application at login.
///
/// Ports `wasLaunchedAtLogin` from [LaunchAtLogin-Modern]:
///
/// ```swift
/// let event = NSAppleEventManager.shared().currentAppleEvent
/// return event?.eventID == kAEOpenApplication
///     && event?.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == keyAELaunchedAsLogInItem
/// ```
///
/// Returns `false` if the event is unavailable or parsing fails.
///
/// Must be called synchronously inside the setup closure: the event is only
/// readable while the launch callback is running.
///
/// [LaunchAtLogin-Modern]: https://github.com/sindresorhus/LaunchAtLogin-Modern
#[cfg(target_os = "macos")]
pub fn launched_by_autostart() -> bool {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};

    // Four-character codes (FourCC / OSType) defined in AppleEvents.h and AERegistry.h.
    const K_AE_OPEN_APPLICATION: u32 = u32::from_be_bytes(*b"oapp");
    const KEY_AE_PROP_DATA: u32 = u32::from_be_bytes(*b"prdt");
    const KEY_AE_LAUNCHED_AS_LOGIN_ITEM: u32 = u32::from_be_bytes(*b"lgit");

    let Some(cls) = AnyClass::get(c"NSAppleEventManager") else {
        return false;
    };
    // SAFETY: Selectors exist on NSAppleEventManager and NSAppleEventDescriptor with
    // matching signatures (AEEventID, AEKeyword, and OSType are UInt32).
    // Nullable object returns are safely handled via Option<Retained<_>>.
    unsafe {
        let mgr: Option<Retained<AnyObject>> = msg_send![cls, sharedAppleEventManager];
        let Some(mgr) = mgr else { return false };
        let event: Option<Retained<AnyObject>> = msg_send![&*mgr, currentAppleEvent];
        let Some(event) = event else { return false };
        let event_id: u32 = msg_send![&*event, eventID];
        if event_id != K_AE_OPEN_APPLICATION {
            return false;
        }
        let prop: Option<Retained<AnyObject>> =
            msg_send![&*event, paramDescriptorForKeyword: KEY_AE_PROP_DATA];
        let Some(prop) = prop else { return false };
        let code: u32 = msg_send![&*prop, enumCodeValue];
        code == KEY_AE_LAUNCHED_AS_LOGIN_ITEM
    }
}

/// Non-macOS: autostart registrations (e.g. Windows Run registry key) pass [`AUTOSTART_ARG`].
#[cfg(not(target_os = "macos"))]
pub fn launched_by_autostart() -> bool {
    std::env::args().any(|a| a == AUTOSTART_ARG)
}

/// Tauri's `App::run()` callback. Dispatches platform-specific system events:
///
/// - macOS:
///   - `ExitRequested`: intercepts implicit exits triggered by closing the last window or Cmd+Q,
///     keeping the app alive in the background/tray. Explicit `app.exit()` calls (`code = Some(_)`)
///     are allowed through.
///   - `Reopen`: restores and focuses the main window when clicking the Dock icon while all windows
///     are hidden, avoiding an unresponsive appearance.
/// - Non-macOS: no-op (closing behavior is handled in `WindowEvent::CloseRequested`; Windows has
///   no Dock or Reopen concept).
#[cfg(target_os = "macos")]
pub fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        // code: None 表示用户触发的退出（关窗 / Cmd+Q）；非 None 是 app.exit(code)，
        // 后者是程序主动退出，不阻拦。配合 MINIMIZE_TO_TRAY 才把窗关变成最小化到托盘。
        //
        // 关窗销毁路线下这个拦截还多了一层含义：主窗口销毁后 app "没有窗口了"，
        // 这里的 prevent_exit 正是它能以纯托盘态活下去的原因。
        tauri::RunEvent::ExitRequested {
            api, code: None, ..
        } if crate::MINIMIZE_TO_TRAY.load(std::sync::atomic::Ordering::Relaxed) => {
            api.prevent_exit();
        }
        // User clicked the app icon in the Dock (macOS Reopen event). When minimized to tray,
        // the app normally switches to Accessory policy without a Dock icon; this serves as a fallback
        // in case the policy switch failed, ensuring the main window is restored or recreated.
        // 不用事件里的 has_visible_windows：AppKit 可能把一直显示着的岛也算进去，那样点 Dock 图标就
        // 叫不回主窗口了
        tauri::RunEvent::Reopen { .. } if !shows_a_window(app) => {
            crate::bootstrap::show_or_recreate_main(app);
        }
        _ => {}
    }
}

/// 有没有显示着的窗口，岛不算。
#[cfg(target_os = "macos")]
fn shows_a_window(app: &tauri::AppHandle) -> bool {
    use tauri::Manager;
    app.webview_windows().iter().any(|(label, window)| {
        label != crate::island::LABEL && window.is_visible().unwrap_or(false)
    })
}

/// Non-macOS: no-op (Reopen is a macOS Dock concept).
#[cfg(not(target_os = "macos"))]
pub fn handle_run_event(_app: &tauri::AppHandle, _event: tauri::RunEvent) {}

/// 任何一个窗口最小化或者还原时调 `on_change`。在主线程上调一次。
///
/// 只有 macOS 要这样听：tao 在 macOS 上最小化、还原时不发窗口事件。Windows 上最小化会发
/// `Resized`，这里什么都不做。
#[cfg(target_os = "macos")]
pub fn on_minimize_changed(on_change: impl Fn() + 'static) {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSWindowDidDeminiaturizeNotification, NSWindowDidMiniaturizeNotification};
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject};

    struct Ivars {
        on_change: Box<dyn Fn()>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "HindsightMinimizeObserver"]
        #[ivars = Ivars]
        struct Observer;

        impl Observer {
            #[unsafe(method(minimizeChanged:))]
            fn minimize_changed(&self, _notification: &NSNotification) {
                (self.ivars().on_change)();
            }
        }
    );

    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("platform: cannot watch minimizing: not on the main thread");
        return;
    };
    let this = Observer::alloc(mtm).set_ivars(Ivars {
        on_change: Box::new(on_change),
    });
    let observer: Retained<Observer> = unsafe { msg_send![super(this), init] };
    let receiver: &AnyObject = &observer;
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: the observer has a `minimizeChanged:` method, and is never released (see below)
    unsafe {
        for name in [
            NSWindowDidMiniaturizeNotification,
            NSWindowDidDeminiaturizeNotification,
        ] {
            center.addObserver_selector_name_object(
                receiver,
                sel!(minimizeChanged:),
                Some(name),
                None,
            );
        }
    }
    // 通知中心不持有观察者：留着它，和 app 一样长
    std::mem::forget(observer);
}

#[cfg(not(target_os = "macos"))]
pub fn on_minimize_changed(_on_change: impl Fn() + 'static) {}

// ───────────── webview 电源纪律：Windows 挂起 / 恢复（其余平台 no-op） ─────────────
//
// 背景：WKWebView 在窗口隐藏/最小化时自带渲染停止 + 亚秒定时器节流（实测），
// WebView2 没有任何自动节流——挂托盘后前端定时器全速照跑。这里用官方
// TrySuspend（Edge 睡眠标签页机制）补齐：脚本整体暂停、渲染进程工作集交给
// OS 回收；变可见时引擎自动恢复，另加显式 Resume 兜底。

/// 是否发出过挂起请求。作用有二：Resized 高频路径上，恢复分支先过这道原子门，
/// 拖拽缩放不会反复跨线程调 COM；挂起任务落地前用户又把窗口调出来了，也靠它放弃。
#[cfg(target_os = "windows")]
static WEBVIEW_SUSPEND_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 窗口隐藏 / 最小化后延迟挂起 webview。
///
/// 延迟 800ms：让隐藏瞬间的在途 IPC 落地，误点隐藏又马上调出时直接放弃。
/// 官方约束：TrySuspend 前 controller 必须不可见——先 SetIsVisible(false)。
/// best-effort：DevTools 打开等睡眠条件不满足时引擎拒绝挂起，只是不省内存，
/// 不影响功能；失败一律降级为日志。
#[cfg(target_os = "windows")]
pub fn schedule_webview_suspend(window: &tauri::WebviewWindow) {
    use std::sync::atomic::Ordering;
    WEBVIEW_SUSPEND_REQUESTED.store(true, Ordering::Relaxed);
    let w = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        if !WEBVIEW_SUSPEND_REQUESTED.load(Ordering::Relaxed) {
            return; // 期间已被 resume_webview_if_suspended 撤销
        }
        let hidden = !w.is_visible().unwrap_or(true) || w.is_minimized().unwrap_or(false);
        if !hidden {
            return;
        }
        let r = w.with_webview(|pw| unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3;
            use webview2_com::TrySuspendCompletedHandler;
            use windows_core::Interface;

            let controller = pw.controller();
            let _ = controller.SetIsVisible(false);
            let Ok(core) = controller.CoreWebView2() else {
                return;
            };
            let Ok(wv3) = core.cast::<ICoreWebView2_3>() else {
                // WebView2 运行时过旧（< 1.0.774）才会走到这，正常渠道不会
                log::warn!("WebView2 无 ICoreWebView2_3，跳过挂起");
                return;
            };
            let handler = TrySuspendCompletedHandler::create(Box::new(|hr, ok| {
                match (hr, ok) {
                    (Ok(()), true) => log::info!("webview 已挂起，渲染进程内存交还 OS"),
                    (Ok(()), false) => {
                        log::info!("webview 挂起被引擎拒绝（睡眠条件不满足），不影响功能");
                    }
                    (Err(e), _) => log::warn!("webview 挂起失败: {e}"),
                }
                Ok(())
            }));
            if let Err(e) = wv3.TrySuspend(&handler) {
                log::warn!("TrySuspend 调用失败: {e}");
            }
        });
        if let Err(e) = r {
            log::warn!("with_webview(挂起) 失败: {e}");
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn schedule_webview_suspend(_window: &tauri::WebviewWindow) {}

/// 显示 / 还原窗口时恢复 webview：SetIsVisible(true)（引擎对变可见的 webview
/// 会自动 resume），再查 IsSuspended 显式 Resume 一次兜底。
/// 从未请求过挂起时只是一次原子读，零开销。
#[cfg(target_os = "windows")]
pub fn resume_webview_if_suspended(window: &tauri::WebviewWindow) {
    use std::sync::atomic::Ordering;
    if !WEBVIEW_SUSPEND_REQUESTED.swap(false, Ordering::Relaxed) {
        return;
    }
    let r = window.with_webview(|pw| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3;
        use windows_core::Interface;

        let controller = pw.controller();
        let _ = controller.SetIsVisible(true);
        let Ok(core) = controller.CoreWebView2() else {
            return;
        };
        let Ok(wv3) = core.cast::<ICoreWebView2_3>() else {
            return;
        };
        let mut suspended = windows_core::BOOL(0);
        if wv3.IsSuspended(&mut suspended).is_ok() && suspended.as_bool() {
            if let Err(e) = wv3.Resume() {
                log::warn!("webview 恢复失败: {e}");
            }
        }
    });
    if let Err(e) = r {
        log::warn!("with_webview(恢复) 失败: {e}");
    }
}

#[cfg(not(target_os = "windows"))]
pub fn resume_webview_if_suspended(_window: &tauri::WebviewWindow) {}

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

// ─────────────────────────────────────────────────────────────
// Machine id: an input of the local key (sync::local_key, ADR-0002).
// The returned bytes must never change: if they do, no stored token can be
// decrypted and every user is signed out.
// ─────────────────────────────────────────────────────────────

/// Returns Windows' `MachineGuid` from the registry, which changes only when
/// Windows is reinstalled.
#[cfg(target_os = "windows")]
pub fn read_machine_id() -> Result<Vec<u8>> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::shared::minwindef::HKEY;
    use winapi::um::winnt::{KEY_READ, KEY_WOW64_64KEY, REG_SZ};
    use winapi::um::winreg::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE};

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    // SAFETY: every winapi call gets the arguments MSDN documents, and `buf`
    // holds a GUID string (38 characters plus the null).
    unsafe {
        let subkey = to_wide("SOFTWARE\\Microsoft\\Cryptography");
        let value = to_wide("MachineGuid");
        let mut hkey: HKEY = std::ptr::null_mut();
        // Without KEY_WOW64_64KEY, a 32-bit process reads SOFTWARE\WOW6432Node,
        // which has no MachineGuid.
        let r = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            0,
            KEY_READ | KEY_WOW64_64KEY,
            &mut hkey,
        );
        if r != 0 {
            return Err(Error::Other(format!(
                "RegOpenKeyExW HKLM\\SOFTWARE\\Microsoft\\Cryptography failed: {r}"
            )));
        }
        let mut buf = [0u16; 128];
        let mut size: u32 = std::mem::size_of_val(&buf) as u32;
        let mut ty: u32 = 0;
        let r = RegQueryValueExW(
            hkey,
            value.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut size,
        );
        RegCloseKey(hkey);
        if r != 0 {
            return Err(Error::Other(format!(
                "RegQueryValueExW MachineGuid failed: {r}"
            )));
        }
        if ty != REG_SZ {
            return Err(Error::Other(format!(
                "MachineGuid has registry type {ty}, expected REG_SZ"
            )));
        }
        // `size` is in bytes and counts the trailing null.
        let chars = (size as usize / 2).saturating_sub(1);
        let s = String::from_utf16_lossy(&buf[..chars]);
        Ok(s.into_bytes())
    }
}

/// Returns the logic board's `IOPlatformUUID`, which changes only with a new
/// logic board.
#[cfg(target_os = "macos")]
pub fn read_machine_id() -> Result<Vec<u8>> {
    let out = std::process::Command::new("ioreg")
        .args(["-d2", "-c", "IOPlatformExpertDevice"])
        .output()
        .map_err(|e| Error::Other(format!("spawn ioreg failed: {e}")))?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "ioreg exited with status {:?}",
            out.status.code()
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Format：`    "IOPlatformUUID" = "ABC-123-..."`
    for line in stdout.lines() {
        if !line.contains("IOPlatformUUID") {
            continue;
        }
        // Split the line by `"` and take the 4th part as the UUID.
        let parts: Vec<&str> = line.split('"').collect();
        if parts.len() >= 4 {
            let uuid = parts[3].trim();
            if !uuid.is_empty() {
                return Ok(uuid.as_bytes().to_vec());
            }
        }
    }
    Err(Error::Other(
        "ioreg output did not contain IOPlatformUUID".to_string(),
    ))
}

/// Returns `/etc/machine-id`, or D-Bus's copy on systems without systemd.
#[cfg(target_os = "linux")]
pub fn read_machine_id() -> Result<Vec<u8>> {
    std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .map(|s| s.trim().as_bytes().to_vec())
        .map_err(|e| Error::Other(format!("read machine-id failed: {e}")))
}

/// Other Unix systems, such as FreeBSD: `/etc/machine-id` if it exists,
/// otherwise an error.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
pub fn read_machine_id() -> Result<Vec<u8>> {
    std::fs::read_to_string("/etc/machine-id")
        .map(|s| s.trim().as_bytes().to_vec())
        .map_err(|e| Error::Other(format!("read /etc/machine-id failed: {e}")))
}
