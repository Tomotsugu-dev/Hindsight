//! 截图 embedding 计算（Phase 1C — 相似度去重）。
//!
//! 用 ONNX Runtime 跑 MobileNet v3 small（torchvision IMAGENET1K_V1 权重，分类头砍掉）
//! 拿 576-dim feature，再 L2 normalize 给余弦阈值去重用（[`crate::ai::dedup`]）。
//!
//! - [`MOBILENET_ONNX`] 编译时嵌入 [`resources/models/mobilenet_v3_small.onnx`]，~3.5 MB
//! - [`MODEL_ID`] 跟 DB schema 的 `screenshot_embeddings.model_id` 字段对齐——升级模型
//!   时换 ID 即可，旧 (path, model_id) 行自然失效不冲突
//! - [`compute_batch`] 是唯一对外入口：传一组路径返回对齐的 embedding 数组
//!
//! Session 用全局 `OnceLock<Mutex<...>>` 懒加载 + 单例：首次调用时从 in-memory bytes
//! 起 session（约 50-100 ms），后续命中即用。Mutex 锁粒度是整个 batch（32 张），
//! summary_runner 串行跑段保证零争抢。
//!
//! ## DLL 依赖（load-dynamic）
//!
//! Cargo feature `load-dynamic` 让 ort 运行时加载 onnxruntime DLL，**不**静态链接。
//! 查找顺序：
//! 1. 环境变量 `ORT_DYLIB_PATH` 指定的绝对路径
//! 2. 跟可执行文件同目录的 `onnxruntime.dll` (Windows) / `libonnxruntime.so` / `.dylib`
//!
//! 部署：Tauri bundle.resources 把 onnxruntime.dll 装进 app dir，自动符合 #2。
//! 开发期：把 DLL 拷到 `target/debug/` 下；缺失时首次调用会 panic 在 ort lib_handle。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;

use image::imageops::FilterType;
use ndarray::Array4;
use ort::session::Session;
use ort::value::TensorRef;
use rayon::prelude::*;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};

/// 编译时嵌入的 ONNX 模型权重（~3.5 MB）。运行时从 in-memory bytes 起 session，
/// 不读盘，方便单文件 ship。
const MOBILENET_ONNX: &[u8] =
    include_bytes!("../../resources/models/mobilenet_v3_small.onnx");

/// 模型标识——跟 DB `screenshot_embeddings.model_id` 字段对齐。
/// 切到 DINOv2 / 其他 backbone 时改这个值；旧 embedding 行 (path, 旧id) 自然失效。
pub const MODEL_ID: &str = "mobilenet-v3-small-imagenet1k_v1";

/// 输出 embedding 维度。MobileNet v3 small 全局池化后是 576。
pub const EMBEDDING_DIM: usize = 576;

/// 一次推理的批量大小——跟 POC 一致。CPU 上 5ms/张，32 张 ~160ms。
const BATCH_SIZE: usize = 32;

/// ImageNet 预处理常量——torchvision IMAGENET1K_V1 transform 直译。
/// resize 短边到 256 后中心裁剪 224，再按 mean/std 归一化。
const RESIZE_SHORT_SIDE: u32 = 256;
const CROP_SIZE: u32 = 224;
const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// 全局单例 session：懒加载，首次调用 [`get_session`] 时初始化。
/// 用 Mutex 而非 RwLock —— `Session::run` 要求 `&mut self`。
static SESSION: OnceLock<Mutex<Session>> = OnceLock::new();

/// 启动时调用：把 onnxruntime DLL 的绝对路径塞进 `ORT_DYLIB_PATH`，让 load-dynamic
/// 在 Tauri prod 包里也能定位到 `<install>/resources/runtime/onnxruntime.dll`。
///
/// 不传 → ort 默认按 "next-to-exe" 搜（dev 模式 `build.rs` 把 DLL 复制到
/// `target/<profile>/`，正好命中默认路径，所以 dev 模式调不调本函数都行）。
///
/// 失败仅打 warn，不中断启动——session 真的起不来时会在 [`get_session`] 抛
/// `EmbeddingFailed`，让总结整段标 error；不影响 capture / 报表等其他子系统。
pub fn init_dylib_path(handle: &AppHandle) {
    let libname: &str = if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    };
    // resource_dir 在 Windows = <install>\resources，macOS = ...Contents/Resources，
    // Linux = /usr/lib/<app>/resources；下面拼 runtime/ 子目录跟 build.rs / bundle 配置一致
    let Ok(res_dir) = handle.path().resource_dir() else {
        log::warn!("init_dylib_path: resource_dir 失败，ort 走默认 next-to-exe 搜索");
        return;
    };
    let candidate = res_dir.join("resources").join("runtime").join(libname);
    if candidate.exists() {
        std::env::set_var("ORT_DYLIB_PATH", &candidate);
        log::info!("ORT_DYLIB_PATH = {}", candidate.display());
        return;
    }
    // dev 模式 resource_dir 不指向 src-tauri/resources（指向 target/debug/...），找不到
    // 是常态——build.rs 已把 DLL 复制到 target/<profile>/，ort 默认搜索能命中。
    log::info!(
        "init_dylib_path: {} 不存在，走 ort 默认 next-to-exe 搜索（dev 模式正常）",
        candidate.display()
    );
}

/// 拿到 session 引用（首次调用时初始化）。
fn get_session() -> Result<&'static Mutex<Session>> {
    if let Some(s) = SESSION.get() {
        return Ok(s);
    }
    let session = Session::builder()
        .map_err(|e| Error::EmbeddingFailed(format!("session builder: {e}")))?
        .commit_from_memory(MOBILENET_ONNX)
        .map_err(|e| Error::EmbeddingFailed(format!("commit_from_memory: {e}")))?;
    // 多线程 race 时只有第一个赢家的 session 存活，其它的被 drop 掉
    let _ = SESSION.set(Mutex::new(session));
    Ok(SESSION.get().expect("session just set"))
}

/// 批量算 embedding：传一组截图路径，返回 L2-normalized 576-dim 向量数组（顺序对齐）。
///
/// 失败语义：任意一张图读盘 / 解码 / 推理失败都向上抛——summary_runner 的段循环
/// 会把整段标 `error`，不悄悄留个空 vec 让后续 dedup 行为诡异。
///
/// 性能：CPU 单线程 ~5 ms/张（MobileNet v3 small + 224x224）。1000 张段 ~5 s，
/// 跟 step 1 vision LLM 比可忽略。GPU EP 留待后续——CPU 已远快于瓶颈。
pub async fn compute_batch(image_paths: &[PathBuf]) -> Result<Vec<Vec<f32>>> {
    if image_paths.is_empty() {
        return Ok(Vec::new());
    }
    let paths: Vec<PathBuf> = image_paths.to_vec();
    // ort + ndarray + image 都是同步阻塞 API，扔 spawn_blocking 不堵 tokio runtime
    tokio::task::spawn_blocking(move || compute_batch_blocking(&paths))
        .await
        .map_err(|e| Error::EmbeddingFailed(format!("spawn_blocking: {e}")))?
}

/// 同步内核：`compute_batch` 的 spawn_blocking 闭包。
fn compute_batch_blocking(paths: &[PathBuf]) -> Result<Vec<Vec<f32>>> {
    let session = get_session()?;
    let mut out: Vec<Vec<f32>> = Vec::with_capacity(paths.len());

    for chunk in paths.chunks(BATCH_SIZE) {
        let batch = preprocess_batch(chunk)?;
        // SessionOutputs 借住 session 的生命周期，必须在锁内把 f32 拷出来再放锁
        let (batch_n, flat) = {
            let mut guard = session
                .lock()
                .map_err(|e| Error::EmbeddingFailed(format!("session mutex poisoned: {e}")))?;
            let input = TensorRef::from_array_view(batch.view())
                .map_err(|e| Error::EmbeddingFailed(format!("tensor from view: {e}")))?;
            let outputs = guard
                .run(ort::inputs![input])
                .map_err(|e| Error::EmbeddingFailed(format!("session run: {e}")))?;
            let (shape, data) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::EmbeddingFailed(format!("extract output: {e}")))?;
            if shape.len() != 2 || shape[1] as usize != EMBEDDING_DIM {
                return Err(Error::EmbeddingFailed(format!(
                    "unexpected output shape: {shape:?}"
                )));
            }
            (shape[0] as usize, data.to_vec())
        };
        for i in 0..batch_n {
            let start = i * EMBEDDING_DIM;
            let end = start + EMBEDDING_DIM;
            out.push(l2_normalize(&flat[start..end]));
        }
    }
    Ok(out)
}

/// 对一组图做完整预处理 → 拼成 (B, 3, 224, 224) 的 f32 ndarray。
///
/// **并行**：rayon 把 batch 内 N 张图同时跑 JPEG 解码 + resize + normalize；
/// 跑实测瓶颈在解码（每张 ~10ms），8 核并行后 batch 32 ≈ 60ms 完成，比串行 ~480ms 快 8x。
///
/// 任一图失败整批失败——上层段循环会标 error 段，不静默丢图。
fn preprocess_batch(paths: &[PathBuf]) -> Result<Array4<f32>> {
    let n = paths.len();
    let pixels: usize = (CROP_SIZE * CROP_SIZE) as usize;

    // 每张图独立算成 3 × H × W flat f32 vec，rayon 并行
    let chunks: Vec<Vec<f32>> = paths
        .par_iter()
        .map(|p| preprocess_one(p))
        .collect::<Result<Vec<_>>>()?;

    // 拼成 (B, 3, 224, 224)：直接拷进底层连续 buffer，省一次 ndarray::stack 拷贝
    let mut flat: Vec<f32> = Vec::with_capacity(n * 3 * pixels);
    for chunk in chunks {
        flat.extend_from_slice(&chunk);
    }
    Array4::from_shape_vec(
        (n, 3, CROP_SIZE as usize, CROP_SIZE as usize),
        flat,
    )
    .map_err(|e| Error::EmbeddingFailed(format!("from_shape_vec: {e}")))
}

/// 单图预处理：load → resize → center crop → normalize → 返回 3×H×W flat f32（CHW 顺序）。
/// 长度恒为 3 × CROP_SIZE × CROP_SIZE = 3 × 224 × 224 = 150528。
fn preprocess_one(path: &Path) -> Result<Vec<f32>> {
    let img = image::open(path).map_err(|e| {
        Error::EmbeddingFailed(format!("open image {}: {e}", path.display()))
    })?;
    let img = img.to_rgb8();
    let (w, h) = (img.width(), img.height());

    // 短边缩到 RESIZE_SHORT_SIDE，长边按比例（torchvision Resize(256) 行为）
    let (new_w, new_h) = if w < h {
        let new_h = (h as f32 * RESIZE_SHORT_SIDE as f32 / w as f32).round() as u32;
        (RESIZE_SHORT_SIDE, new_h)
    } else {
        let new_w = (w as f32 * RESIZE_SHORT_SIDE as f32 / h as f32).round() as u32;
        (new_w, RESIZE_SHORT_SIDE)
    };
    let resized = image::imageops::resize(&img, new_w, new_h, FilterType::Triangle);

    // 中心裁剪到 CROP_SIZE × CROP_SIZE
    let crop_x = (new_w.saturating_sub(CROP_SIZE)) / 2;
    let crop_y = (new_h.saturating_sub(CROP_SIZE)) / 2;
    let cropped =
        image::imageops::crop_imm(&resized, crop_x, crop_y, CROP_SIZE, CROP_SIZE).to_image();

    // CHW 顺序 + ImageNet 归一化：先按通道分别填，避免 (i,c,y,x) 内层 c 跳跃
    let cs = CROP_SIZE as usize;
    let mut out = vec![0.0_f32; 3 * cs * cs];
    for c in 0..3 {
        let plane_offset = c * cs * cs;
        let mean = IMAGENET_MEAN[c];
        let std = IMAGENET_STD[c];
        for y in 0..CROP_SIZE {
            let row_offset = plane_offset + y as usize * cs;
            for x in 0..CROP_SIZE {
                let px = cropped.get_pixel(x, y);
                let v = px[c] as f32 / 255.0;
                out[row_offset + x as usize] = (v - mean) / std;
            }
        }
    }
    Ok(out)
}

/// L2 归一化：让向量模长为 1，余弦相似度 = dot product。
fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_normalize_unit_norm() {
        let v = vec![3.0_f32, 4.0];
        let n = l2_normalize(&v);
        let mag: f32 = n.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((mag - 1.0).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_zero_vector_passthrough() {
        let v = vec![0.0_f32, 0.0, 0.0];
        let n = l2_normalize(&v);
        assert_eq!(n, v);
    }
}
