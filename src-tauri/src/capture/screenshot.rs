use std::path::{Path, PathBuf};

use chrono::Local;
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ExtendedColorType, ImageEncoder};

use crate::error::{Error, Result};

/// 截当前激活窗口 → 缩放到 `max_width × max_height`（保持比例 + letterbox 透明）→
/// JPEG 编码 → 写到 `dir/<HHMMSS_NNN>.jpg`。返回写入的绝对路径。
/// 同步图像处理放在 `spawn_blocking` 里跑，不堵 Tokio runtime。
pub async fn capture_active_window(
    dir: PathBuf,
    max_width: u32,
    max_height: u32,
    jpeg_quality: u8,
) -> Result<Option<String>> {
    let result = tokio::task::spawn_blocking(move || {
        capture_blocking(&dir, max_width, max_height, jpeg_quality)
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
) -> Result<Option<String>> {
    let rgba = match grab_focused_image() {
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
fn grab_focused_image() -> Option<image::RgbaImage> {
    #[cfg(target_os = "macos")]
    {
        let pid = macos_frontmost_pid()?;
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
        if pid > 0 { Some(pid as u32) } else { None }
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
