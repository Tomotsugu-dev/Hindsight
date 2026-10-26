//! macOS：`NSWorkspace` 拿前台 app，xcap 补标题，取不到标题时用 ScreenCaptureKit 再补一次。
//!
//! 不直接用 xcap 拿 app 归属：xcap 在多屏 / 多 Space 下窗口枚举不全（v0.5.61 诊断版
//! 实测：副屏 Hindsight 状态下，主屏 Chrome 的窗口完全不在 `xcap::Window::all()` 里），
//! 导致 PID-match 失败 → tick 跳过 → 上一个 Hindsight session 一直挂着不结束 → 所有
//! 时间被错记到 Hindsight。

use std::sync::Arc;

use super::pick::{pick_main_window_title, WindowCandidate};
use super::{basename, resolve_exe_path, FocusSource, WindowInfo};
use crate::error::{Error, Result};

/// 返回 macOS 的焦点来源。
pub(super) fn detect() -> Arc<dyn FocusSource> {
    Arc::new(MacFocus)
}

struct MacFocus;

impl FocusSource for MacFocus {
    fn focused_window(&self) -> Result<WindowInfo> {
        if let Some(info) = resolve_focused_window() {
            return Ok(info);
        }
        // 极端 fallback：NSWorkspace 返 nil（锁屏 / 登录窗 等）→ 走老 xcap heuristic
        xcap_focused_window()
    }

    fn recover_title(&self, pid: u32) -> Option<String> {
        recover_title(pid)
    }
}

/// 老 xcap heuristic：在 xcap 的窗口列表里找有焦点的那个。只在 NSWorkspace 拿不到
/// 前台 app 时用（见 [`MacFocus::focused_window`]）。
///
/// # Errors
///
/// xcap 列不出窗口，或者没有窗口有焦点。
fn xcap_focused_window() -> Result<WindowInfo> {
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
/// [`crate::capture::bundle::canonicalize_to_parent_bundle`] 折叠到最外层父 bundle，让
/// activities 行的 `process_name` 始终是用户认识的那个名字。
fn resolve_focused_window() -> Option<WindowInfo> {
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
            crate::capture::bundle::canonicalize_to_parent_bundle(&raw_name, raw_path.as_deref());

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

/// SCK 取标题失败后的退避时长。没授 Screen Recording 权限的机器上
/// `SCShareableContent::get()` 每次都要失败，而它单次开销 ~100ms（CGWindowList
/// 只要 ~1ms），不退避的话每个 tick 都白烧。
const SCK_TITLE_BACKOFF: std::time::Duration = std::time::Duration::from_secs(300);

static SCK_TITLE_BLOCKED_UNTIL: std::sync::Mutex<Option<std::time::Instant>> =
    std::sync::Mutex::new(None);

/// 便宜路径（CGWindowList / xcap）取不到标题时的兜底：走截图那条 SCK 枚举。
///
/// **为什么值得多花这 ~100ms**：SCK 跨 Space 跨显示器枚举，且按面积挑主窗口，
/// 不受 z 序和"窗口不在当前 Space"影响。实测口径——同一 tick、同一 PID 下，
/// 标题取空的会话里有 87.6%（Chrome 93.2%）截图是成功的，说明 SCK 当时找得到
/// 那个窗口，那么从它上面读 title 就能拿到。
///
/// **为什么不直接当主路径**：SCK 单次枚举 ~100ms、CGWindowList ~1ms，而后者在
/// 稳态下就够用（实测 45 轮忠实复刻 + 299 次采样 0 失败）。只在失败时付费。
///
/// 调用方必须在 `spawn_blocking` 里跑（同 [`crate::capture::screenshot_macos`]）。
fn recover_title(pid: u32) -> Option<String> {
    use screencapturekit::shareable_content::SCShareableContent;

    {
        let blocked = SCK_TITLE_BLOCKED_UNTIL.lock().ok()?;
        if blocked.is_some_and(|until| std::time::Instant::now() < until) {
            return None;
        }
    }

    // SCK 内部大量 autorelease 临时对象；同截图路径包一层 pool，别让它们沉在
    // 常驻的 blocking 线程上。
    objc2::rc::autoreleasepool(|_| {
        let content = match SCShareableContent::get() {
            Ok(c) => c,
            Err(e) => {
                // 没权限 / SCK 不可用：退避，别每 tick 白烧 100ms
                log::debug!(
                    "SCK 补取标题失败，退避 {}s：{e:?}",
                    SCK_TITLE_BACKOFF.as_secs()
                );
                if let Ok(mut blocked) = SCK_TITLE_BLOCKED_UNTIL.lock() {
                    *blocked = Some(std::time::Instant::now() + SCK_TITLE_BACKOFF);
                }
                return None;
            }
        };

        let cands: Vec<WindowCandidate> = content
            .windows()
            .iter()
            .map(|w| {
                let frame = w.frame();
                WindowCandidate {
                    pid: w
                        .owning_application()
                        .map(|app| app.process_id() as u32)
                        .unwrap_or(0),
                    layer: w.window_layer(),
                    on_screen: w.is_on_screen(),
                    area: frame.width * frame.height,
                    title: w.title().unwrap_or_default(),
                }
            })
            .collect();

        pick_main_window_title(&cands, pid)
    })
}

#[cfg(test)]
mod tests {
    /// 真机自检（默认 ignore，CI 不跑）：对当前前台 app 跑一遍 SCK 补取，
    /// 打印拿到的标题。改动 SCK 绑定 / 升级 screencapturekit 后手动验一次：
    /// `cargo test --lib sck_recover_title_smoke -- --ignored --nocapture`
    /// 需要跑测试的终端本身有 Screen Recording 权限，否则只会看到 None。
    #[test]
    #[ignore = "需要真实窗口 + Screen Recording 权限，手动跑"]
    fn sck_recover_title_smoke() {
        use objc2_app_kit::NSWorkspace;
        let pid = objc2::rc::autoreleasepool(|_| {
            let ws = NSWorkspace::sharedWorkspace();
            ws.frontmostApplication().map(|a| a.processIdentifier())
        })
        .unwrap_or(0);
        assert!(pid > 0, "拿不到前台 app");
        println!(
            "前台 pid={pid} SCK 标题={:?}",
            super::recover_title(pid as u32)
        );
    }
}
