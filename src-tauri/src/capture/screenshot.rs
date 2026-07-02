use std::path::{Path, PathBuf};

use chrono::Local;
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ExtendedColorType, ImageEncoder};

use crate::error::{Error, Result};

/// 截当前激活窗口 → 缩放到 `max_width × max_height`（保持比例 + letterbox 透明）→
/// JPEG 编码 → 写到 `dir/<HHMMSS_NNN>.jpg`。返回写入的绝对路径。
/// 同步图像处理放在 `spawn_blocking` 里跑，不堵 Tokio runtime。
///
/// `expected_pid`：tick 开始时解析出的焦点进程。隐私过滤是基于那一刻的窗口信息
/// 判定的，而截图在几百 ms 后才发生——若此刻前台已切到别的（可能命中隐私过滤的）
/// 应用，直接放弃本次截图，避免"过滤按 A 判、镜头拍到 B"的 TOCTOU。0 = 不校验。
pub async fn capture_active_window(
    dir: PathBuf,
    max_width: u32,
    max_height: u32,
    jpeg_quality: u8,
    expected_pid: u32,
) -> Result<Option<String>> {
    let result = tokio::task::spawn_blocking(move || {
        capture_blocking(&dir, max_width, max_height, jpeg_quality, expected_pid)
    })
    .await
    .map_err(|e| Error::Capture(format!("screenshot task join: {e}")))?;
    result
}

fn capture_blocking(
    dir: &Path,
    max_width: u32,
    max_height: u32,
    jpeg_quality: u8,
    expected_pid: u32,
) -> Result<Option<String>> {
    let rgba = match grab_focused_image(expected_pid) {
        Some(img) => img,
        None => return Ok(None),
    };

    let img = DynamicImage::ImageRgba8(rgba);
    let normalized = fit_within(img, max_width, max_height);
    let rgb = normalized.to_rgb8();

    let now = Local::now();
    let date_dir = now.format("%Y-%m-%d").to_string();
    let file_name = format!("{}.jpg", now.format("%H%M%S_%3f"));
    let target_dir = dir.join(&date_dir);
    std::fs::create_dir_all(&target_dir)?;
    let target = target_dir.join(&file_name);

    let file = std::fs::File::create(&target)?;
    let encoder = JpegEncoder::new_with_quality(file, jpeg_quality.clamp(30, 95));
    encoder
        .write_image(&rgb, rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
        .map_err(|e| Error::Capture(format!("jpeg encode: {e}")))?;

    Ok(Some(target.to_string_lossy().to_string()))
}

/// 拿当前前台 app 的截图——macOS 走 ScreenCaptureKit（focused window only），
/// 其它平台走 xcap heuristic。失败返 None，调用方跳过该 tick。
///
/// `expected_pid != 0` 时校验"现在的前台还是 tick 判定隐私时的那个进程"，
/// 已经切走就放弃（宁可少一张图，不拍错人）。
fn grab_focused_image(expected_pid: u32) -> Option<image::RgbaImage> {
    #[cfg(target_os = "macos")]
    {
        let pid = macos_frontmost_pid()?;
        if expected_pid != 0 && pid != expected_pid {
            log::debug!("跳过截图：前台已从 pid={expected_pid} 切到 pid={pid}（TOCTOU 防护）");
            return None;
        }
        match super::screenshot_macos::capture_focused_window(pid) {
            Ok(img) => Some(img),
            Err(e) => {
                log::debug!("SCK 截图失败: {e}");
                None
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let windows = xcap::Window::all().ok()?;
        let focused = windows.iter().find(|w| w.is_focused().unwrap_or(false))?;
        if expected_pid != 0 {
            let pid = focused.pid().unwrap_or(0);
            if pid != 0 && pid != expected_pid {
                log::debug!("跳过截图：前台已从 pid={expected_pid} 切到 pid={pid}（TOCTOU 防护）");
                return None;
            }
        }
        focused.capture_image().ok()
    }
}

/// macOS：调 `NSWorkspace.frontmostApplication()` 拿系统层最前 app 的 PID。
/// 这是跨屏 / 跨 Space 都正确的"用户当前在用的 app"信号——比 xcap 的 is_focused()
/// 在多屏场景下可靠很多。
#[cfg(target_os = "macos")]
fn macos_frontmost_pid() -> Option<u32> {
    use objc2_app_kit::NSWorkspace;
    // 在 tokio blocking 线程上调 AppKit，没有 ambient autoreleasepool 会导致
    // NSRunningApplication 等 autorelease 临时对象堆积。
    objc2::rc::autoreleasepool(|_| {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = app.processIdentifier();
        if pid > 0 {
            Some(pid as u32)
        } else {
            None
        }
    })
}

/// 整窗保留：等比缩放使其装入 max_w × max_h，超过任一上限才缩，否则保持原始尺寸。
fn fit_within(img: DynamicImage, max_w: u32, max_h: u32) -> DynamicImage {
    if max_w == 0 || max_h == 0 {
        return img;
    }
    if img.width() <= max_w && img.height() <= max_h {
        return img;
    }
    img.resize(max_w, max_h, image::imageops::FilterType::Triangle)
}
