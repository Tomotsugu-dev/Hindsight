//! 焦点窗口：现在前台是哪个应用、哪个窗口。采集服务只通过 [`FocusSource`] 读取，
//! 不知道底下是哪个系统在回答。
//!
//! 每个系统一份实现，编译时选定：
//! - Windows → [`windows`]
//! - macOS → [`macos`]
//! - Linux → [`linux`]
//!
//! 全平台都是轮询：采集服务每个 tick 调一次 [`FocusSource::focused_window`]。
//! [`FocusSource::watch`] 让实现可以在焦点变化时提前叫醒 tick 循环，但数据仍然从
//! `focused_window` 拉，tick 只有一条路径。

use std::sync::Arc;

use serde::Serialize;
use sysinfo::{Pid, System};
use tokio::sync::Notify;

use crate::error::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

/// 当前焦点窗口的元信息（用于判断是否切焦点 / 写 activities 行）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub app_name: String,
    pub title: String,
    pub app_path: Option<String>,
    /// 解析焦点时的进程 PID；0 = 未知。截图路径用它做"同一进程"校验：
    /// 隐私过滤基于 tick 开始时的窗口信息判定，截图却在几百 ms 后才拍——
    /// 中间焦点切到隐私应用的话，不带校验会把隐私画面拍下来挂到错的会话上。
    pub pid: u32,
}

/// 在一个系统上读取焦点窗口。采集服务持有 [`detect`] 返回的那一个。
pub trait FocusSource: Send + Sync {
    /// 读取此刻的焦点窗口。会阻塞，所以必须快：tick 直接调用它。
    ///
    /// # Errors
    ///
    /// 没有窗口在前台，或问不到系统。tick 跳过这一轮。
    fn focused_window(&self) -> Result<WindowInfo>;

    /// [`Self::focused_window`] 返回空标题时，对进程 `pid` 再慢一点地补取一次标题。
    /// 调用方在 `spawn_blocking` 里跑它。None = 没有更好的标题。
    fn recover_title(&self, _pid: u32) -> Option<String> {
        None
    }

    /// 焦点或标题变化时调用 [`FocusWaker::wake`]，直到返回的守卫被丢弃。
    /// None = 这个实现不能推送，单靠轮询照常工作。
    fn watch(&self, _waker: FocusWaker) -> Option<WatchGuard> {
        None
    }
}

/// 返回本系统的焦点来源。采集服务创建时调用一次。
pub fn detect() -> Arc<dyn FocusSource> {
    imp::detect()
}

/// The state of the Hindsight GNOME extension. Only GNOME Wayland needs it to read the focused
/// window; the frontend prompts the user when it is not [`Self::Ready`].
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GnomeExtensionStatus {
    /// The extension is not needed, or it is running
    Ready,
    /// The extension is installed, but GNOME has not found it yet: log out and back in once
    NeedsRelogin,
    /// The extension is not installed
    Missing,
    /// GNOME has found it, but it is off: [`enable_gnome_extension`] turns it on
    Off,
    /// The user turned off all extensions in GNOME
    AllOff,
    /// This GNOME version cannot run the extension
    Unsupported,
    /// The extension hit an error
    Failed,
}

/// Returns the state of the Hindsight GNOME extension. Asks GNOME Shell, so the caller runs it in
/// `spawn_blocking`.
#[cfg(target_os = "linux")]
pub fn gnome_extension_status() -> GnomeExtensionStatus {
    linux::gnome_extension_status()
}

/// Turns on the Hindsight GNOME extension. Asks GNOME Shell, so the caller runs it in
/// `spawn_blocking`.
///
/// # Errors
///
/// GNOME Shell cannot be asked, or it did not turn the extension on:
/// [`crate::error::Error::Capture`]
#[cfg(target_os = "linux")]
pub fn enable_gnome_extension() -> Result<()> {
    linux::enable_gnome_extension()
}

/// 叫醒采集循环，让它立刻跑一次 tick，而不是等满轮询周期。
///
/// tick 运行期间来的多次叫醒合并成一次：`Notify` 只保留一个待处理标记，所以 tick
/// 结束后最多立刻再跑一次。可以从任何线程调用。
#[derive(Clone, Default)]
pub struct FocusWaker(Arc<Notify>);

impl FocusWaker {
    /// 请求立刻跑一次 tick。
    // 目前还没有实现会推送，第一个是 GNOME 扩展的 FocusChanged 信号，
    // 接上后去掉这个 allow。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn wake(&self) {
        self.0.notify_one();
    }

    /// 在下一次 [`Self::wake`] 时完成；上次调用之后已经有过叫醒的话，立刻完成。
    pub async fn woken(&self) {
        self.0.notified().await;
    }
}

/// 被丢弃时停止 [`FocusSource::watch`]。采集运行期间由 tick 任务持有。
pub struct WatchGuard(Option<Box<dyn FnOnce() + Send>>);

impl WatchGuard {
    /// `stop` 在守卫被丢弃时运行一次。
    // 同 FocusWaker::wake：第一个实现 watch 的来源接上后去掉这个 allow。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(stop: impl FnOnce() + Send + 'static) -> Self {
        Self(Some(Box::new(stop)))
    }
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            stop();
        }
    }
}

/// xcap 在某些情况下（特别是 UWP 应用）会把完整路径塞进 app_name。
/// 取最后一段斜杠后的内容作为真正的进程名。
fn basename(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(idx) = trimmed.rfind(['\\', '/']) {
        trimmed[idx + 1..].to_string()
    } else {
        trimmed.to_string()
    }
}

// macOS 用 NSWorkspace 给的 bundle 路径，不调它
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn resolve_exe_path(pid: u32) -> Option<String> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        sysinfo::ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
    );
    sys.process(Pid::from_u32(pid))
        .and_then(|p| p.exe().map(|p| p.to_string_lossy().to_string()))
}

/// 判断捕获到的 app 名是不是"调试字符串残片"这类垃圾。
///
/// AppKit / xcap 在进程正处于启动或退出的瞬间，偶尔会把内部对象描述的碎片当
/// 名字给出来（实测库里出现过 `"17969442 pid=49230 ]"`、`"8607797 pid=58750 ]"`）。
/// 真实应用名不会包含 `" pid="`，也不会是 `<...>` 形式的 ObjC 描述串。
/// 记录这种行的后果：应用列表 / 配对页出现无图标、无标题、名字是乱码的幽灵应用。
pub(crate) fn is_garbage_window_name(name: &str) -> bool {
    let t = name.trim();
    t.contains(" pid=") || (t.starts_with('<') && t.ends_with('>'))
}

/// 判断某个 process_name 是不是"系统占位进程"——锁屏 / 屏保 这种用户**显然不在用电脑**
/// 但 macOS 仍然把它当前台 app 返回的进程。capture 看到这些应该立刻 seal 当前会话
/// 不再累计时长，等同于"用户挂机"。
///
/// 不依赖 [`crate::capture::idle::IdleSource::idle_secs`] —— macOS 锁屏后 idle 计数有时回 0 / 不增，
/// 单靠系统 idle 信号会让 17 分钟锁屏被记成 17 分钟使用。
///
/// 黑名单只列"无歧义占位"那几个：
/// - `loginwindow` —— 锁屏 / 登录窗 / 登出确认
/// - `ScreenSaverEngine` / `ScreenSaverAgent` —— 屏保
///
/// **不**列 SecurityAgent（用户在输密码 = 真活动）/ CoreServicesUIAgent
/// （系统模态对话框 = 用户在交互），那些算"用户在交互"不应跳过。
pub(crate) fn is_system_idle_proxy(app_name: &str) -> bool {
    matches!(
        app_name,
        "loginwindow" | "ScreenSaverEngine" | "ScreenSaverAgent"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// 等 `woken` 最多这么久；超时 = 没有待处理的叫醒。
    const NO_WAKE: Duration = Duration::from_millis(50);

    #[tokio::test]
    async fn wake_before_waiting_is_not_lost() {
        // 实现方可能在 tick 运行时（循环还没开始等）就叫醒——这次叫醒必须留到下一轮
        let waker = FocusWaker::default();
        waker.wake();
        assert!(tokio::time::timeout(NO_WAKE, waker.woken()).await.is_ok());
    }

    #[tokio::test]
    async fn wakes_during_a_tick_collapse_into_one() {
        // Alt-Tab 一扫是一串叫醒：tick 结束后只该再跑一次，不能排成一队
        let waker = FocusWaker::default();
        for _ in 0..5 {
            waker.wake();
        }
        assert!(tokio::time::timeout(NO_WAKE, waker.woken()).await.is_ok());
        assert!(tokio::time::timeout(NO_WAKE, waker.woken()).await.is_err());
    }

    #[tokio::test]
    async fn no_wake_means_the_loop_keeps_waiting() {
        // 不能推送的实现从不叫醒：循环只能靠轮询的 sleep 醒来
        let waker = FocusWaker::default();
        assert!(tokio::time::timeout(NO_WAKE, waker.woken()).await.is_err());
    }

    #[test]
    fn watch_guard_stops_once_on_drop() {
        let stops = Arc::new(AtomicUsize::new(0));
        let guard = {
            let stops = Arc::clone(&stops);
            WatchGuard::new(move || {
                stops.fetch_add(1, Ordering::SeqCst);
            })
        };
        assert_eq!(stops.load(Ordering::SeqCst), 0, "还没丢弃就停止了");
        drop(guard);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn sources_without_push_return_no_guard() {
        // 默认的 watch：不支持推送，返回 None，采集服务照常轮询
        struct PollOnly;
        impl FocusSource for PollOnly {
            fn focused_window(&self) -> Result<WindowInfo> {
                Err(crate::error::Error::Capture("unused".into()))
            }
        }
        assert!(PollOnly.watch(FocusWaker::default()).is_none());
        assert_eq!(PollOnly.recover_title(1), None);
    }

    #[test]
    fn garbage_window_name_matches_debug_fragments() {
        // 实际数据库里出现过的残片
        assert!(is_garbage_window_name("17969442 pid=49230 ]"));
        assert!(is_garbage_window_name("8607797 pid=58750 ]"));
        // ObjC 描述串形态
        assert!(is_garbage_window_name(
            "<NSRunningApplication: 0x600001b30340>"
        ));
        // 正常应用名不能误伤
        assert!(!is_garbage_window_name("WeChat"));
        assert!(!is_garbage_window_name("Visual Studio Code"));
        assert!(!is_garbage_window_name("Foo [Beta]"));
        assert!(!is_garbage_window_name("百度网盘"));
    }

    #[test]
    fn is_system_idle_proxy_matches_lockscreen_processes() {
        assert!(is_system_idle_proxy("loginwindow"));
        assert!(is_system_idle_proxy("ScreenSaverEngine"));
        assert!(is_system_idle_proxy("ScreenSaverAgent"));
    }

    #[test]
    fn is_system_idle_proxy_does_not_match_user_facing_apps() {
        // 用户在用电脑的常见前台 app
        assert!(!is_system_idle_proxy("WeChat"));
        assert!(!is_system_idle_proxy("Chrome"));
        assert!(!is_system_idle_proxy("Code"));
        // SecurityAgent / CoreServicesUIAgent 是"用户在交互"——不算挂机
        assert!(!is_system_idle_proxy("SecurityAgent"));
        assert!(!is_system_idle_proxy("CoreServicesUIAgent"));
        // 空 / 未知 / 自身 = 正常 app 路径
        assert!(!is_system_idle_proxy(""));
        assert!(!is_system_idle_proxy("hindsight"));
    }

    #[test]
    fn garbage_window_name_matches_whitespace_wrapped_fragments() {
        // AppKit 给出的残片有时带前后空白——判定内部先 trim，包一层空白不能漏网。
        // ObjC 描述串的 <...> 判定依赖 starts_with/ends_with，不 trim 的话
        // 前导空格会让 starts_with('<') 失败，残片就混进应用列表了。
        assert!(is_garbage_window_name(
            "  <NSRunningApplication: 0x600001b30340>  "
        ));
        assert!(is_garbage_window_name("\t<NSWindow: 0x7f8a3c40>\n"));
        // pid= 残片包空白同样要判垃圾
        assert!(is_garbage_window_name("  17969442 pid=49230 ]  "));
    }

    #[test]
    fn garbage_window_name_allows_normal_names() {
        // 含数字的真实应用名——数字本身不是垃圾特征
        assert!(!is_garbage_window_name("7-Zip"));
        assert!(!is_garbage_window_name("1Password 7"));
        // 非 ASCII 应用名
        assert!(!is_garbage_window_name("微信"));
        // "Rapid" 里含字母序列 pid，但垃圾特征是 " pid="（带空格带等号）——
        // 只 contains("pid") 的实现会误杀这类正常名，这里锁死正确边界
        assert!(!is_garbage_window_name("Rapid Photo Downloader"));
        // 只有半边尖括号不是完整 ObjC 描述串
        assert!(!is_garbage_window_name("<incomplete"));
        // 尖括号在名字中间（如 beta 标记）不满足"整串被 <> 包裹"
        assert!(!is_garbage_window_name("app <beta>"));
        // 空串走的是"名字缺失"路径，不该被当垃圾判定拦下
        assert!(!is_garbage_window_name(""));
    }

    #[test]
    fn system_idle_proxy_requires_whole_name_match() {
        // 黑名单必须整名匹配——substring 匹配会把恰好含 "loginwindow" 的
        // 第三方进程误判成锁屏，导致真实使用时长被当挂机 seal 掉。
        assert!(!is_system_idle_proxy("myloginwindow"));
        assert!(!is_system_idle_proxy("loginwindow2"));
        assert!(!is_system_idle_proxy("Loginwindow Helper"));
    }

    #[test]
    fn system_idle_proxy_excludes_desktop_shell_processes() {
        // Windows 桌面本体（Progman / WorkerW 窗口类）和 explorer 不属于这份
        // 黑名单——桌面前台由 platform::is_desktop_foreground 单独检测，语义
        // 是"被动观看豁免不适用"而非"用户挂机"。两条路径职责不同，这里锁住
        // 边界防止有人图方便把它们塞进 idle proxy 黑名单。
        assert!(!is_system_idle_proxy("Progman"));
        assert!(!is_system_idle_proxy("WorkerW"));
        assert!(!is_system_idle_proxy("explorer"));
        assert!(!is_system_idle_proxy("explorer.exe"));
    }

    #[test]
    fn basename_strips_uwp_full_path() {
        // xcap 对 UWP 应用会把完整安装路径塞进 app_name——三种斜杠形态都要能剥
        assert_eq!(
            basename(r"C:\Program Files\WindowsApps\Microsoft.Todos_2.54\Todo.exe"),
            "Todo.exe"
        );
        assert_eq!(
            basename("/Applications/Safari.app/Contents/MacOS/Safari"),
            "Safari"
        );
        // 混合斜杠：取"最后一个任意方向斜杠"之后的段
        assert_eq!(basename(r"C:\Users/foo\AppData/App.exe"), "App.exe");
    }

    #[test]
    fn basename_passes_plain_names_through() {
        // 无斜杠 = 已是纯进程名，原样返回
        assert_eq!(basename("WeChat"), "WeChat");
        // 前后空白要清掉——窗口系统给的名字偶尔带缝隙空白
        assert_eq!(basename("  Code  "), "Code");
        // 空串不 panic、返回空串
        assert_eq!(basename(""), "");
    }

    #[test]
    fn resolve_exe_path_nonexistent_pid_returns_none() {
        // macOS 默认 pid 上限 99999（Linux 默认 4194304），远超上限的 PID
        // 必然不存在——sysinfo 查不到进程时必须返 None 而不是空串/panic
        assert_eq!(resolve_exe_path(999_999_999), None);
    }

    #[test]
    fn resolve_exe_path_self_resolves_current_exe() {
        // 用测试进程自己做活体样本：PID 一定存在，exe 一定可解析。
        // canonicalize 两边再比——sysinfo 与 std 可能一个给符号链接一个给实路径
        let pid = std::process::id();
        let resolved = resolve_exe_path(pid).expect("自身进程必须能解析出 exe 路径");
        let resolved = std::path::Path::new(&resolved)
            .canonicalize()
            .expect("解析出的路径必须真实存在");
        let expected = std::env::current_exe()
            .expect("current_exe 可用")
            .canonicalize()
            .expect("current_exe 可 canonicalize");
        assert_eq!(resolved, expected);
    }

    #[test]
    fn window_info_serializes_to_camel_case() {
        // 前端 TS 侧按 camelCase 读字段——这是跨语言契约，rename_all 被删/改会
        // 让前端拿到 undefined 而不报错，必须用测试锁死
        let info = WindowInfo {
            app_name: "WeChat".into(),
            title: "聊天".into(),
            app_path: Some("/Applications/WeChat.app".into()),
            pid: 4242,
        };
        let v = serde_json::to_value(&info).expect("序列化必须成功");
        assert_eq!(v["appName"], "WeChat");
        assert_eq!(v["title"], "聊天");
        assert_eq!(v["appPath"], "/Applications/WeChat.app");
        assert_eq!(v["pid"], 4242);
        // snake_case 字段名不得出现
        let obj = v.as_object().expect("必须是 JSON object");
        assert!(!obj.contains_key("app_name"));
        assert!(!obj.contains_key("app_path"));

        // app_path = None 序列化为 null（字段仍在，不是被省略）——前端按
        // `appPath ?? fallback` 处理，字段整个消失和 null 行为不同
        let info_none = WindowInfo {
            app_name: "x".into(),
            title: String::new(),
            app_path: None,
            pid: 0,
        };
        let v2 = serde_json::to_value(&info_none).expect("序列化必须成功");
        assert!(v2.as_object().expect("object").contains_key("appPath"));
        assert!(v2["appPath"].is_null());
    }
}
