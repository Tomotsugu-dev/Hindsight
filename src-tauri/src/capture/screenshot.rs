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

/// 拿当前前台 app 的截图：优先窗口，拿不到（macOS 多屏下 xcap 经常看不到主屏
/// app 的窗口）就回退到"主显示器"整屏；都不行返 None。
fn grab_focused_image() -> Option<image::RgbaImage> {
    // macOS 走 NSWorkspace 系统 API 拿真正的前台 app PID；其它平台 PID 拿不到也无所谓，
    // 走 xcap heuristic 找 focused 窗口。
    #[cfg(target_os = "macos")]
    let frontmost_pid = macos_frontmost_pid();
    #[cfg(not(target_os = "macos"))]
    let frontmost_pid: Option<u32> = None;

    if let Ok(windows) = xcap::Window::all() {
        let focused = windows.iter().find(|w| match frontmost_pid {
            Some(p) => w.pid().ok() == Some(p),
            None => w.is_focused().unwrap_or(false),
        });
        if let Some(w) = focused {
            match w.capture_image() {
                Ok(img) => return Some(img),
                Err(e) => log::debug!("窗口截图失败: {e}"),
            }
        }
    }

    // macOS 回退：xcap 看不到主屏的窗口（多屏 / 多 Space 已知问题）→ 截"主显示器"
    // 整屏。`is_primary()` 在 macOS 上跟着 menubar 走，通常就是用户当前在用的那块屏。
    #[cfg(target_os = "macos")]
    {
        if let Ok(monitors) = xcap::Monitor::all() {
            if let Some(m) = monitors.iter().find(|m| m.is_primary().unwrap_or(false)) {
                match m.capture_image() {
                    Ok(img) => return Some(img),
                    Err(e) => log::debug!("主显示器整屏截图失败: {e}"),
                }
            }
        }
    }

    None
}

/// 调 `NSWorkspace.frontmostApplication()` 拿系统层最前 app 的 PID。
/// 跟 [`crate::capture::window`] 那个同名函数功能一样，只是这里不想拉跨模块依赖
/// 就再写一份；3 行实现，复制成本远低于多套一层 pub 接口。
#[cfg(target_os = "macos")]
fn macos_frontmost_pid() -> Option<u32> {
    use objc2_app_kit::NSWorkspace;
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = app.processIdentifier();
    if pid > 0 { Some(pid as u32) } else { None }
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
