use serde::Serialize;
use sysinfo::{Pid, System};

use crate::error::{Error, Result};

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

/// 拉当前焦点窗口的 [`WindowInfo`]。取不到（屏幕权限缺失 / 没有窗口在前 等）
/// 返回 `Err`，调用方 log debug 跳过本次 tick。
///
/// macOS 走 `NSWorkspace.frontmostApplication` 拿 app 元数据——xcap 在多屏 / 多
/// Space 下窗口枚举不全（v0.5.61 诊断版实测：副屏 Hindsight 状态下，主屏 Chrome
/// 的窗口完全不在 `xcap::Window::all()` 里），导致 PID-match 失败 → tick 跳过 →
/// 上一个 Hindsight session 一直挂着不结束 → 所有时间被错记到 Hindsight。
/// 现在 macOS 完全跳过 xcap 拿 app 归属，title 还是 best-effort 从 xcap 拿。
pub fn current_window() -> Result<WindowInfo> {
    // macOS 走 NSWorkspace 直接拿前台 app 的元数据——xcap 在多屏 / 多 Space 下
    // 窗口枚举不全（v0.5.61 诊断版实测：副屏 Hindsight 的状态下，主屏 Chrome 的
    // 窗口完全不在 xcap::Window::all() 里）。NSRunningApplication 给 localizedName
    // + bundleURL 足够定位 app 归属；title 仍尝试从 xcap 拿，拿不到就空着——
    // 总比把所有时间错记到 Hindsight 强。
    #[cfg(target_os = "macos")]
    {
        if let Some(info) = macos_resolve_focused_window() {
            return Ok(info);
        }
        // 极端 fallback：NSWorkspace 返 nil（锁屏 / 登录窗 等）→ 走老 xcap heuristic
    }

    let windows = xcap::Window::all().map_err(|e| Error::Capture(e.to_string()))?;

    let focused = windows
        .iter()
        .find(|w| w.is_focused().unwrap_or(false))
        .ok_or_else(|| Error::Capture("没有焦点窗口".into()))?;

    let raw_name = focused.app_name().unwrap_or_default().to_string();
    let app_name = basename(&raw_name);
    let title = focused.title().unwrap_or_default().to_string();
    let pid = focused.pid().unwrap_or(0);

    let app_path = if pid > 0 { resolve_exe_path(pid) } else { None };

    Ok(WindowInfo {
        app_name,
        title,
        app_path,
        pid,
    })
}

/// macOS：通过 NSWorkspace 拿系统层 frontmost app 的 (name, pid, bundle path)，
/// 再用 PID filter xcap 窗口列表拿 title（拿不到无所谓，title 空着仍能正确归属
/// 到对应 app）。返 None = NSWorkspace 这层失败，调用方落回老 xcap 路径。
///
/// **helper / mini-program 子 bundle 归一**：WeChat 的 mini-program 跑在嵌套
/// `WeChatAppEx.app` 里、Claude / Chrome 这种 Electron app 把渲染进程打成
/// `Claude Helper (Renderer).app`，NSWorkspace 直接把这些当独立 app 返回 ——
/// `localizedName` 会是 "WeChatAppEx" 而非 "WeChat"。这里调
/// [`super::bundle::canonicalize_to_parent_bundle`] 折叠到最外层父 bundle，让
/// activities 行的 `process_name` 始终是用户认识的那个名字。
#[cfg(target_os = "macos")]
fn macos_resolve_focused_window() -> Option<WindowInfo> {
    use objc2_app_kit::NSWorkspace;
    // tokio worker 线程上没有 ambient autoreleasepool，AppKit/CG 内部 autorelease 的
    // 临时对象（NSString / NSURL / NSPathStore2 / CGWindow 列表的 NSValue 等）会一直
    // 沉在线程上不释放 —— 5s tick × 长期 uptime 累计低 MB / 小时的 RSS 漂移。
    objc2::rc::autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        let pid_i32 = app.processIdentifier();
        if pid_i32 <= 0 {
            return None;
        }
        let pid = pid_i32 as u32;

        let raw_name = app
            .localizedName()
            .map(|s| s.to_string())
            .unwrap_or_default();
        if raw_name.trim().is_empty() {
            return None;
        }

        let raw_path = app
            .bundleURL()
            .and_then(|url| url.path())
            .map(|s| s.to_string());

        let (app_name, app_path) =
            super::bundle::canonicalize_to_parent_bundle(&raw_name, raw_path.as_deref());

        // title 是 nice-to-have——xcap 多屏下经常拿不到主屏 app 的窗口，那就空着
        let title = xcap::Window::all()
            .ok()
            .and_then(|ws| {
                ws.into_iter()
                    .find(|w| w.pid().ok() == Some(pid))
                    .and_then(|w| w.title().ok())
            })
            .unwrap_or_default();

        Some(WindowInfo {
            app_name: basename(&app_name),
            title,
            app_path,
            pid,
        })
    })
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
/// 不依赖 [`crate::platform::idle_secs`] —— macOS 锁屏后 idle 计数有时回 0 / 不增，
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
