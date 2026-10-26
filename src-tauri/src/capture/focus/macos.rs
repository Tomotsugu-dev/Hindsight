//! On macOS, query NSWorkspace for the foreground app. First read its title from
//! a window visible on screen (xcap); if that fails, try ScreenCaptureKit.
//!
//! Do not identify the foreground app from xcap's window list: it only lists
//! windows visible on screen, so the foreground app's window may be missing
//! when using multiple displays or Spaces.

use std::sync::Arc;

use super::pick::{pick_main_window_title, WindowCandidate};
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

#[cfg(test)]
mod tests {
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
