//! Screenshots: captures the focused window and saves it as a JPEG. Each system only grabs the
//! focused window's pixels (`grab_focused`), chosen at compile time; scaling, skipping tiny
//! images, encoding and writing the file are here, the same on every system.
//!
//! Whether to take one is up to [`policy`] (the privacy rules): the capture service asks it before
//! calling [`capture_active_window`].

use std::path::{Path, PathBuf};

use chrono::Local;
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ExtendedColorType, ImageEncoder};

use crate::error::{Error, Result};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
pub mod policy;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

/// Screenshots smaller than this size in either width or height are not saved:
/// the macOS OCR engine rejects images that are too small.
const MIN_SIDE_PX: u32 = 16;

/// Captures the focused window and saves it as `dir/<date>/<HHMMSS_mmm>.jpg`, returning the file's
/// path; `Ok(None)` when nothing was captured.
///
/// `expected_pid` is the focused process when privacy was checked; if the foreground has changed
/// by the time of the screenshot, none is taken. 0 = no check.
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
    let rgba = match imp::grab_focused(expected_pid) {
        Some(img) => img,
        None => return Ok(None),
    };
    save_image(dir, rgba, max_width, max_height, jpeg_quality)
}

/// Resize the image and save it as a JPEG, returning the saved path.
/// If the resized image is too small, it is not saved and `None` is returned.
fn save_image(
    dir: &Path,
    rgba: image::RgbaImage,
    max_width: u32,
    max_height: u32,
    jpeg_quality: u8,
) -> Result<Option<String>> {
    let img = DynamicImage::ImageRgba8(rgba);
    let normalized = fit_within(img, max_width, max_height);
    if normalized.width() < MIN_SIDE_PX || normalized.height() < MIN_SIDE_PX {
        log::debug!(
            "Screenshot skipped: {}×{} is too small",
            normalized.width(),
            normalized.height()
        );
        return Ok(None);
    }
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

/// 整窗保留：等比缩放使其装入 max_w × max_h，超过任一上限才缩，否则保持原始尺寸。
/// Lanczos3:存档是 OCR 的输入,缩放质量直接决定小字是否可读(Triangle 会糊笔画)。
fn fit_within(img: DynamicImage, max_w: u32, max_h: u32) -> DynamicImage {
    if max_w == 0 || max_h == 0 {
        return img;
    }
    if img.width() <= max_w && img.height() <= max_h {
        return img;
    }
    img.resize(max_w, max_h, image::imageops::FilterType::Lanczos3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_dir() -> PathBuf {
        std::env::temp_dir().join(format!("hindsight-shot-{}", uuid::Uuid::new_v4().simple()))
    }

    /// 1×1 的图不保存，连日期目录都不建。
    #[test]
    fn tiny_image_is_not_saved() {
        let dir = fresh_dir();
        let saved = save_image(&dir, image::RgbaImage::new(1, 1), 1280, 800, 80).unwrap();
        assert_eq!(saved, None);
        assert!(!dir.exists());
    }

    /// 缩小后才变小的也不保存：2000×20 装进 1280×800，高只剩 12 像素。
    #[test]
    fn image_too_small_after_scaling_is_not_saved() {
        let dir = fresh_dir();
        let saved = save_image(&dir, image::RgbaImage::new(2000, 20), 1280, 800, 80).unwrap();
        assert_eq!(saved, None);
        assert!(!dir.exists());
    }

    /// 正好 16×16 的照常保存。
    #[test]
    fn image_at_the_limit_is_saved() {
        let dir = fresh_dir();
        let saved = save_image(&dir, image::RgbaImage::new(16, 16), 1280, 800, 80)
            .unwrap()
            .expect("16×16 应该保存");
        assert!(Path::new(&saved).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
