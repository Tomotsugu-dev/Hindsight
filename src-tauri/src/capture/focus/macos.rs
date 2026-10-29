//! On macOS, query NSWorkspace for the foreground app. First read its title from
//! a window visible on screen (xcap); if that fails, try ScreenCaptureKit.
//!
//! Do not identify the foreground app from xcap's window list: it only lists
//! windows visible on screen, so the foreground app's window may be missing
//! when using multiple displays or Spaces.

use std::sync::Arc;

use super::{basename, FocusSource, WindowInfo};
use crate::error::{Error, Result};

/// Returns the macOS focus source.
pub(super) fn detect() -> Arc<dyn FocusSource> {
    Arc::new(MacFocus)
}

struct MacFocus;

impl FocusSource for MacFocus {
    fn focused_window(&self) -> Result<WindowInfo> {
        resolve_focused_window()
            .ok_or_else(|| Error::Capture("No foreground app was provided by NSWorkspace".into()))
    }

    fn recover_title(&self, pid: u32) -> Option<String> {
        recover_title(pid)
    }
}

/// Reads the foreground app (NSWorkspace) and its title from a window visible
/// on screen (inexpensive, but often returns an empty string).
///
/// If the title is empty, [`recover_title`] tries again. Returns `None` if
/// NSWorkspace did not provide a foreground app.
fn resolve_focused_window() -> Option<WindowInfo> {
    use objc2_app_kit::NSWorkspace;
    // Temporary objects returned by system APIs are released when the
    // autorelease pool is drained. Background threads have no regular pool
    // drains, so without this pool they accumulate on the thread and use
    // progressively more memory.
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

        // The title is best-effort: xcap often cannot find the app's window
        // across multiple displays, so leave it empty in that case.
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

/// Backoff duration after SCK fails to retrieve a title. Without Screen
/// Recording permission, `SCShareableContent::get()` fails every time, so
/// without a backoff each tick would make a futile attempt.
const SCK_TITLE_BACKOFF: std::time::Duration = std::time::Duration::from_secs(300);

static SCK_TITLE_BLOCKED_UNTIL: std::sync::Mutex<Option<std::time::Instant>> =
    std::sync::Mutex::new(None);

/// Tries to read the title again with ScreenCaptureKit, choosing the main
/// window with [`pick_main_window_title`].
///
/// It lists all windows (including those on other Spaces and hidden windows)
/// and is dozens of times slower than CGWindowList. Call it only when the
/// inexpensive path returns an empty title, and run it inside
/// `spawn_blocking`. After a failure, return `None` without retrying during
/// [`SCK_TITLE_BACKOFF`].
fn recover_title(pid: u32) -> Option<String> {
    use screencapturekit::shareable_content::SCShareableContent;

    // During the backoff period, skip the SCK request that failed last time.
    {
        let blocked = SCK_TITLE_BLOCKED_UNTIL.lock().ok()?;
        if blocked.is_some_and(|until| std::time::Instant::now() < until) {
            return None;
        }
    }

    // Temporary objects returned by system APIs are released when the
    // autorelease pool is drained. Background threads have no regular pool
    // drains, so without this pool they accumulate on the thread and use
    // progressively more memory.
    objc2::rc::autoreleasepool(|_| {
        let content = match SCShareableContent::get() {
            Ok(c) => c,
            // TODO: screencapturekit 2.1's get() converts errors to strings,
            // so we cannot distinguish missing permission (-3801) from a
            // transient failure and must back off for both. Once they can be
            // distinguished, stop retrying for this process when permission
            // is missing, and retry on the next tick after a transient failure.
            Err(e) => {
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

/// Window properties used to choose the main window.
#[derive(Debug, Clone, PartialEq)]
struct WindowCandidate {
    pid: u32,
    layer: i32,
    on_screen: bool,
    area: f64,
    title: String,
}

/// Returns the title of the process's main window.
///
/// Must choose the same window as [`crate::capture::screenshot`] on macOS, or the
/// title and the screenshot come from different windows.
fn pick_main_window_title(cands: &[WindowCandidate], pid: u32) -> Option<String> {
    let main = cands
        .iter()
        .filter(|w| w.pid == pid && w.layer == 0 && w.on_screen)
        .max_by(|a, b| {
            a.area
                .partial_cmp(&b.area)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    let title = main.title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

#[cfg(test)]
mod tests {
    use super::{pick_main_window_title, WindowCandidate};

    /// fixture 尺寸取自实测：主窗口 2128×1114，无名窄条 2128×30 / 1710×34。
    fn cand(pid: u32, layer: i32, on_screen: bool, w: f64, h: f64, title: &str) -> WindowCandidate {
        WindowCandidate {
            pid,
            layer,
            on_screen,
            area: w * h,
            title: title.into(),
        }
    }

    #[test]
    fn main_window_title_beats_nameless_strips() {
        // 复现「无标题」的真实场景：同一 PID 名下一堆无名窄条 + 一个真窗口，
        // 按 z 序取第一个会读到空串，按面积取则拿到真标题。
        let cands = vec![
            cand(760, 0, true, 2128.0, 30.0, ""),
            cand(760, 0, true, 1710.0, 34.0, ""),
            cand(
                760,
                0,
                true,
                2128.0,
                1114.0,
                "Issue #24 · Tomotsugu-dev/Hindsight",
            ),
            cand(760, 0, true, 2128.0, 30.0, ""),
        ];
        assert_eq!(
            pick_main_window_title(&cands, 760).as_deref(),
            Some("Issue #24 · Tomotsugu-dev/Hindsight")
        );
    }

    #[test]
    fn main_window_title_ignores_other_processes() {
        let cands = vec![
            cand(1, 0, true, 3000.0, 2000.0, "别的 app 的大窗口"),
            cand(760, 0, true, 1200.0, 800.0, "自己的窗口"),
        ];
        assert_eq!(
            pick_main_window_title(&cands, 760).as_deref(),
            Some("自己的窗口")
        );
    }

    #[test]
    fn main_window_title_skips_non_normal_layer_and_offscreen() {
        // layer≠0 = dock / menubar / 系统模态之流；on_screen=false = 其它 Space
        // 或最小化。两者都不该被当成主窗口（与截图路径的过滤一致）。
        let cands = vec![
            cand(760, 103, true, 4000.0, 3000.0, "提示气泡"),
            cand(760, 0, false, 5000.0, 4000.0, "另一个 Space 的窗口"),
            cand(760, 0, true, 800.0, 600.0, "当前窗口"),
        ];
        assert_eq!(
            pick_main_window_title(&cands, 760).as_deref(),
            Some("当前窗口")
        );
    }

    #[test]
    fn main_window_title_none_when_genuinely_untitled() {
        // QQ音乐那种:主窗口本身就没标题，三套 API 都返回空串——救不回来，
        // 返回 None 让调用方保持"无标题"，别拿窄条的空串冒充"取到了"。
        let cands = vec![
            cand(760, 0, true, 1049.0, 709.0, "   "),
            cand(760, 0, true, 300.0, 200.0, "小面板"),
        ];
        assert_eq!(pick_main_window_title(&cands, 760), None);
    }

    #[test]
    fn main_window_title_none_when_no_candidate() {
        assert_eq!(pick_main_window_title(&[], 760), None);
        let cands = vec![cand(999, 0, true, 100.0, 100.0, "别人的")];
        assert_eq!(pick_main_window_title(&cands, 760), None);
    }

    #[test]
    fn main_window_title_trims_whitespace() {
        let cands = vec![cand(760, 0, true, 900.0, 700.0, "  带空格的标题  ")];
        assert_eq!(
            pick_main_window_title(&cands, 760).as_deref(),
            Some("带空格的标题")
        );
    }

    /// Manual smoke test (ignored by default and not run in CI): tries SCK title
    /// recovery for the current foreground app and prints the result. Run it
    /// manually after changing the SCK bindings or upgrading screencapturekit:
    /// `cargo test --lib sck_recover_title_smoke -- --ignored --nocapture`
    /// The terminal running the test needs Screen Recording permission;
    /// otherwise the result will be `None`.
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
