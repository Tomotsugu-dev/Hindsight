//! 段内截图选帧 + base64 编码（Phase 1B-γ）。
//!
//! - [`pick_frames`] 给一组按时间排序的截图路径，等距下采样到 `max` 张
//! - [`to_data_uri`] 读盘 → 可选 resize → JPEG → base64 → `data:image/jpeg;base64,...`
//!
//! Phase 1B-γ 阶段不做 dHash 去重；这块留给 Phase 1C（用 settings.ai.hash_threshold
//! / hash_window_minutes 做时间窗内汉明距离聚类）。当前实现是最简单的等距时间采样。

use std::io::Cursor;
use std::path::Path;

use base64::Engine;
use image::ImageFormat;

use crate::error::{Error, Result};

/// 等距下采样：从一组已经按时间排序的元素中取 `max` 个。
///
/// 行为：
/// - `max == 0` → 返回空 Vec（调用方判断要不要给纯文本兜底）
/// - `items.len() <= max` → 全要，原样返回
/// - 否则 → 等距索引 `i * len / max` 取 max 个
///
/// 泛型化（`T: Clone`）支持 `Vec<String>` 路径和 `Vec<ScreenshotMeta>` 元数据两种调用。
/// 不做 dedup：γ 阶段相邻相似帧也送 LLM，反正 vision 模型自己会忽略冗余。
/// dHash 在 Phase 1C 加。
pub fn pick_frames<T: Clone>(items: Vec<T>, max: usize) -> Vec<T> {
    if max == 0 {
        return Vec::new();
    }
    let n = items.len();
    if n <= max {
        return items;
    }
    (0..max).map(|i| items[i * n / max].clone()).collect()
}

/// 读截图文件 → JPEG bytes → base64 → data URI 字符串。
///
/// `max_dim`：
/// - `0` → 不缩放，直接读原盘字节做 base64（最快路径，截图本来就 fit 过）
/// - `> 0` → 解码 + 长边缩到 max_dim 像素 + 重新 JPEG 编码（默认质量 75）
///
/// 失败统一映射成 `Error::Other`，让 summary.rs 的循环能 continue 跳过坏文件
/// 而不是一段卡死。
pub async fn to_data_uri(path: &Path, max_dim: u32) -> Result<String> {
    let path = path.to_path_buf();
    // image crate 是同步阻塞 API，放 spawn_blocking 不堵 tokio runtime
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        if max_dim == 0 {
            return std::fs::read(&path)
                .map_err(|e| Error::Other(format!("读截图失败 {}: {e}", path.display())));
        }

        let img = image::open(&path)
            .map_err(|e| Error::Other(format!("解码截图失败 {}: {e}", path.display())))?;

        let (w, h) = (img.width(), img.height());
        let img = if w.max(h) > max_dim {
            // resize 保持比例：image::imageops::FilterType::Triangle 比 Lanczos 快很多，
            // 视觉差距对 vision LLM 输入而言可忽略
            img.resize(max_dim, max_dim, image::imageops::FilterType::Triangle)
        } else {
            img
        };

        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Jpeg)
            .map_err(|e| Error::Other(format!("编码 JPEG 失败：{e}")))?;
        Ok(buf)
    })
    .await
    .map_err(|e| Error::Other(format!("spawn_blocking 失败：{e}")))??;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:image/jpeg;base64,{}", b64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_frames_basic() {
        let paths: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
        // max=0 空
        assert!(pick_frames(paths.clone(), 0).is_empty());
        // 不足 max 全要
        assert_eq!(pick_frames(paths.clone(), 20).len(), 10);
        // 等距取 5 张：索引 0,2,4,6,8
        let picked = pick_frames(paths.clone(), 5);
        assert_eq!(picked, vec!["p0", "p2", "p4", "p6", "p8"]);
        // 取 1 张：第 0 张
        assert_eq!(pick_frames(paths, 1), vec!["p0"]);
    }
}
