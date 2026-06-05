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
}
