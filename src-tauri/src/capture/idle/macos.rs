//! macOS：CoreGraphics 读键鼠空闲和显示器睡眠，IOKit 读显示防睡断言。

use std::sync::Arc;

use super::IdleSource;

/// 返回 macOS 的来源。
pub(super) fn detect() -> Arc<dyn IdleSource> {
    Arc::new(MacIdle)
}

struct MacIdle;

impl IdleSource for MacIdle {
    /// `CGEventSourceSecondsSinceLastEventType`：合并所有输入源（trackpad / 蓝牙鼠标 /
    /// 外接键盘），不分前台 / 后台 app。
    fn idle_secs(&self) -> u64 {
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
        let s = unsafe {
            CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT)
        };
        if s.is_finite() && s >= 0.0 {
            s as u64
        } else {
            0
        }
    }

    /// 主显示器睡眠 = 息屏（CoreGraphics 直接可查，无需通知线程）。
    /// 锁屏/屏保已由 capture 侧的前台占位进程判定（loginwindow / ScreenSaverEngine）
    /// 覆盖，这里不重复。
    fn screen_unavailable(&self) -> bool {
        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            fn CGMainDisplayID() -> u32;
            fn CGDisplayIsAsleep(display: u32) -> i32;
        }
        // SAFETY: CoreGraphics 公开 C API，任意线程可调，无指针传递。
        unsafe { CGDisplayIsAsleep(CGMainDisplayID()) != 0 }
    }

    /// `IOPMCopyAssertionsStatus` 里的 `PreventUserIdleDisplaySleep` 断言。
    fn display_keepawake_active(&self) -> bool {
        use std::ffi::c_void;
        type CFTypeRef = *const c_void;
        type CFDictionaryRef = *const c_void;
        type CFStringRef = *const c_void;
        #[link(name = "IOKit", kind = "framework")]
        extern "C" {
            fn IOPMCopyAssertionsStatus(assertions_status: *mut CFDictionaryRef) -> i32;
        }
        extern "C" {
            fn CFDictionaryGetValue(dict: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
            fn CFStringCreateWithCString(
                alloc: *const c_void,
                c_str: *const std::os::raw::c_char,
                encoding: u32,
            ) -> CFStringRef;
            fn CFNumberGetValue(number: CFTypeRef, the_type: i32, value_ptr: *mut c_void) -> bool;
            fn CFRelease(cf: CFTypeRef);
        }
        const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
        const KCF_NUMBER_INT_TYPE: i32 = 9;
        // 只认用户态播放器申请的 PreventUserIdleDisplaySleep;
        // InternalPreventDisplaySleep 是系统内部断言,不代表媒体播放。
        const ASSERTION: &[u8] = b"PreventUserIdleDisplaySleep\0";
        unsafe {
            let mut dict: CFDictionaryRef = std::ptr::null();
            if IOPMCopyAssertionsStatus(&mut dict) != 0 || dict.is_null() {
                return false;
            }
            let key = CFStringCreateWithCString(
                std::ptr::null(),
                ASSERTION.as_ptr() as *const std::os::raw::c_char,
                KCF_STRING_ENCODING_UTF8,
            );
            let mut active = false;
            if !key.is_null() {
                let val = CFDictionaryGetValue(dict, key);
                if !val.is_null() {
                    let mut level: i32 = 0;
                    if CFNumberGetValue(
                        val,
                        KCF_NUMBER_INT_TYPE,
                        &mut level as *mut i32 as *mut c_void,
                    ) {
                        active = level > 0;
                    }
                }
                CFRelease(key);
            }
            CFRelease(dict);
            active
        }
    }
}
