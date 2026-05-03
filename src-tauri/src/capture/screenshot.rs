use std::path::{Path, PathBuf};

use chrono::Local;
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ExtendedColorType, ImageEncoder};

use crate::error::{Error, Result};

pub async fn capture_active_window(
    dir: PathBuf,
    target_width: u32,
    target_height: u32,
    jpeg_quality: u8,
) -> Result<Option<String>> {
    let result = tokio::task::spawn_blocking(move || {
        capture_blocking(&dir, target_width, target_height, jpeg_quality)
    })
    .await
    .map_err(|e| Error::Other(format!("截图任务异常: {e}")))?;
    result
}

fn capture_blocking(
    dir: &Path,
    target_width: u32,
    target_height: u32,
    jpeg_quality: u8,
) -> Result<Option<String>> {
    let windows = xcap::Window::all().map_err(|e| Error::Capture(e.to_string()))?;
    let focused = match windows.iter().find(|w| w.is_focused().unwrap_or(false)) {
        Some(w) => w,
        None => return Ok(None),
    };

    let rgba = match focused.capture_image() {
        Ok(img) => img,
        Err(e) => {
            log::debug!("截图失败: {e}");
            return Ok(None);
        }
    };

    let img = DynamicImage::ImageRgba8(rgba);
    let normalized = fit_cover_crop(img, target_width, target_height);
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
        .map_err(|e| Error::Other(format!("JPEG 编码: {e}")))?;

    Ok(Some(target.to_string_lossy().to_string()))
}

/// 缩放到刚好覆盖 target，再从中心裁出精确 target 尺寸。
/// 不变形，超出部分对称裁掉。
fn fit_cover_crop(img: DynamicImage, target_w: u32, target_h: u32) -> DynamicImage {
    if target_w == 0 || target_h == 0 {
        return img;
    }
    let src_w = img.width().max(1);
    let src_h = img.height().max(1);
    let scale_w = target_w as f64 / src_w as f64;
    let scale_h = target_h as f64 / src_h as f64;
    let scale = scale_w.max(scale_h);
    let new_w = ((src_w as f64 * scale).ceil() as u32).max(target_w);
    let new_h = ((src_h as f64 * scale).ceil() as u32).max(target_h);
    let resized = img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle);
    let crop_x = new_w.saturating_sub(target_w) / 2;
    let crop_y = new_h.saturating_sub(target_h) / 2;
    resized.crop_imm(crop_x, crop_y, target_w, target_h)
}
