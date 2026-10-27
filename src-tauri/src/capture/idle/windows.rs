//! Windows：`GetLastInputInfo` 读键鼠空闲，系统通知读息屏和锁屏，执行态读显示防睡请求。

use std::sync::Arc;

use super::IdleSource;

/// 返回 Windows 的来源。
pub(super) fn detect() -> Arc<dyn IdleSource> {
    Arc::new(WindowsIdle)
}

struct WindowsIdle;

impl IdleSource for WindowsIdle {
    /// `GetLastInputInfo` + `GetTickCount` 算 ms 差。
    /// `wrapping_sub` 处理 49 天溢出（GetTickCount u32 ms 大约 49.7 天回绕）。
    fn idle_secs(&self) -> u64 {
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

    /// 息屏与锁屏来自常驻消息窗口线程接收的系统通知
    /// （`GUID_CONSOLE_DISPLAY_STATE` 电源通知 + WTS 会话锁通知），首次调用时
    /// 惰性启动该线程；屏保用 `SPI_GETSCREENSAVERRUNNING` 即时轮询。
    /// 通知注册失败只降级（恒返 false = 回到今天的行为），不影响其它功能。
    fn screen_unavailable(&self) -> bool {
        screen_state::ensure_watcher();
        screen_state::display_off()
            || screen_state::session_locked()
            || screen_state::screensaver_running()
    }

    /// CallNtPowerInformation(SystemExecutionState) 读系统聚合执行态,
    /// 普通权限可用(不需要 powercfg /requests 的管理员位)。ES_DISPLAY_REQUIRED
    /// 位 = 有进程正在申请显示防睡。
    fn display_keepawake_active(&self) -> bool {
        use winapi::um::powerbase::CallNtPowerInformation;
        use winapi::um::winnt::SystemExecutionState;
        const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;
        let mut state: u32 = 0;
        let rc = unsafe {
            CallNtPowerInformation(
                SystemExecutionState,
                std::ptr::null_mut(),
                0,
                &mut state as *mut u32 as *mut _,
                std::mem::size_of::<u32>() as u32,
            )
        };
        rc == 0 && (state & ES_DISPLAY_REQUIRED) != 0
    }
}

/// Windows 息屏/锁屏状态源：隐藏消息窗口线程 + 两个 AtomicBool。
mod screen_state {
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
