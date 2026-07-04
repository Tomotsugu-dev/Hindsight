//! MobileCLIP2-S2(ONNX)图像编码器——屏幕记忆 L4 的视觉传感器。
//!
//! 只做一件事:图 → 512 维 L2 归一化嵌入。聚簇/新颖度判定在
//! `memory::clusters`,文本编码器(P2 搜图用)不在本模块、不预下载。
//!
//! 模型:HuggingFace `RuteNL/MobileCLIP2-S2-OpenCLIP-ONNX` 的
//! `visual.onnx` + `visual.onnx.data`(外部权重,须同目录)。
//! 预处理(官方 open_clip_config):RGB、整图缩放 256×256、/255(mean 0 std 1)。
//! 注:官方是"短边缩放+中心裁剪",截图场景中心裁剪会扔掉约四成宽度,
//! 这里改为整图缩放(轻微形变)——聚簇只关心同源帧的相对相似度,
//! 所有帧同一形变不影响判定,而保住全局版面信息。

use std::path::PathBuf;
use std::sync::Mutex;

use image::DynamicImage;
use ndarray::Array4;
use ort::session::Session;
use ort::value::TensorRef;

use crate::error::{Error, Result};

/// 输入边长(官方 image_size)
const VIS_SIZE: u32 = 256;
/// 嵌入维数(官方 embed_dim)
pub const EMBED_DIM: usize = 512;

/// 模型目录:`<data_root>/ai/visual/`。
pub fn model_dir() -> PathBuf {
    crate::storage::db_path_dir()
        .map(|p| p.join("ai").join("visual"))
        .unwrap_or_else(|_| PathBuf::from("ai").join("visual"))
}

pub struct VisualEngine {
    session: Mutex<Session>,
}

impl VisualEngine {
    /// 从 [`model_dir`] 加载会话。onnxruntime 缺失时与嵌入/OCR 同一条引导链路。
    pub fn load() -> Result<Self> {
        if !crate::ai::embedding_runtime::is_installed().unwrap_or(false) {
            return Err(Error::EmbeddingRuntimeMissing);
        }
        let session = Session::builder()
            .and_then(|b| b.with_intra_threads(crate::ai::ocr::ort_threads()))
            .and_then(|b| b.with_memory_pattern(false))
            .and_then(|b| b.commit_from_file(model_dir().join("visual.onnx")))
            .map_err(|e| Error::Ocr(format!("加载 visual.onnx 失败: {e}")))?;
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    /// 图 → 512 维 L2 归一化嵌入。输入形状恒定(256×256),无形状爆炸问题。
    pub fn embed(&self, img: &DynamicImage) -> Result<Vec<f32>> {
        let rgb = image::imageops::resize(
            &img.to_rgb8(),
            VIS_SIZE,
            VIS_SIZE,
            image::imageops::FilterType::Triangle,
        );
        let mut input = Array4::<f32>::zeros((1, 3, VIS_SIZE as usize, VIS_SIZE as usize));
        for (x, y, p) in rgb.enumerate_pixels() {
            for c in 0..3 {
                input[[0, c, y as usize, x as usize]] = p[c] as f32 / 255.0;
            }
        }

        let mut session = self
            .session
            .lock()
            .map_err(|e| Error::Ocr(format!("visual mutex poisoned: {e}")))?;
        let tensor = TensorRef::from_array_view(input.view())
            .map_err(|e| Error::Ocr(format!("visual tensor: {e}")))?;
        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|e| Error::Ocr(format!("visual run: {e}")))?;
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::Ocr(format!("visual output: {e}")))?;
        if shape.iter().product::<i64>() as usize != EMBED_DIM {
            return Err(Error::Ocr(format!("visual 输出形状异常: {shape:?}")));
        }

        // L2 归一化:聚簇的余弦相似度就变成简单点积
        let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
        Ok(data.iter().map(|v| v / norm).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真模型冒烟:同图嵌入 = 1.0,异图 < 同图。需要模型三件套 + onnxruntime。
    /// 跑法:`cargo test --release --lib visual::tests::real_embed_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_embed_smoke() {
        let dylib = crate::ai::embedding_runtime::dylib_path().unwrap();
        std::env::set_var("ORT_DYLIB_PATH", &dylib);
        let engine = VisualEngine::load().unwrap();

        // 合成两张内容迥异的图:纯色渐变 vs 棋盘格
        let grad = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(320, 240, |x, _| {
            image::Rgb([(x % 256) as u8, 80, 160])
        }));
        let checker = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(320, 240, |x, y| {
            if (x / 16 + y / 16) % 2 == 0 {
                image::Rgb([240, 240, 240])
            } else {
                image::Rgb([16, 16, 16])
            }
        }));

        let t0 = std::time::Instant::now();
        let a = engine.embed(&grad).unwrap();
        let b = engine.embed(&grad).unwrap();
        let c = engine.embed(&checker).unwrap();
        println!("3 次嵌入耗时 {:?}", t0.elapsed());

        let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(a, b)| a * b).sum::<f32>();
        let same = dot(&a, &b);
        let diff = dot(&a, &c);
        println!("同图余弦 {same:.4} | 异图余弦 {diff:.4}");
        assert!(same > 0.999, "同图应该恒等");
        assert!(diff < same - 0.05, "异图应显著低于同图");
    }
}
