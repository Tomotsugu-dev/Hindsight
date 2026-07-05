//! PP-OCRv5(ONNX)文字识别引擎——屏幕记忆 L2 的誊写员。
//!
//! 单模型原生识别 简中/繁中/英/日,混排直认,无语言检测环节
//! (docs/design/screen-memory.md §3 L2)。det(文本检测) + rec(行识别)两个
//! ONNX 会话,跑在与嵌入模型同一个 onnxruntime(load-dynamic)上。
//!
//! 模型文件(det.onnx / rec.onnx / dict.txt)放在 `<data_root>/ai/ocr/`,
//! 来源:HuggingFace `PaddlePaddle/PP-OCRv5_mobile_{det,rec}_onnx` + PaddleOCR
//! 官方 `ppocrv5_dict.txt`。由消化 worker 首跑时下载(见 memory::digest)。
//!
//! 预处理/后处理参数来自官方 inference.yml:
//! - det:BGR、mean/std ImageNet、缩放长边 ≤ [`DET_LIMIT_SIDE`] 且宽高取整到 /32;
//!   DB 后处理 thresh=0.3 / box_thresh=0.6 / unclip=1.5
//! - rec:BGR、(x/255−0.5)/0.5、高 48 宽动态(≤3200,超长行切段分别识别)
//! - CTC:类 0 = blank,1..=N → dict[i−1],N+1 → 空格

use std::path::PathBuf;
use std::sync::Mutex;

use image::{DynamicImage, RgbImage};
use ndarray::Array4;
use ort::session::Session;
use ort::value::TensorRef;

use crate::error::{Error, Result};

/// det 输入长边上限。官方运营点 960 面向照片;屏幕截图上小字(14-16px)在 960
/// 档会缩到检测不稳,取 1920(QHD 存档 2560→1920,14px 字仍有 ~10px)。/32 对齐后喂模型。
const DET_LIMIT_SIDE: u32 = 1920;
/// DB 二值化阈值 / 框置信度阈值 / 扩框比(官方 inference.yml 定案)
const DET_THRESH: f32 = 0.3;
const DET_BOX_THRESH: f32 = 0.6;
const DET_UNCLIP: f32 = 1.5;
/// rec 输入高;宽 = 高 × 纵横比,超过上限的超长行切段识别
const REC_H: u32 = 48;
const REC_MAX_W: u32 = 3200;
/// rec 批大小:按宽度排序后 8 条行一批,批内 pad 到最大宽。
/// 相比逐行调用(一帧 50-100 次 session.run)是最大的纯速度杠杆,无质量损失。
const REC_BATCH: usize = 8;
/// rec 张量宽的阶梯:批宽向上取整到阶梯值,让输入形状种类有界——
/// onnxruntime 的内存池按形状留块且只增不还,宽度连续变化会让内存
/// 随长跑无界增长(实测 505 帧从 1.0GB 涨到 1.4GB)。付出 ~10% padding
/// 计算,换内存曲线封顶。
const REC_W_LADDER: [u32; 8] = [160, 320, 640, 960, 1280, 1600, 2240, 3200];

fn ladder_w(w: u32) -> u32 {
    REC_W_LADDER
        .iter()
        .copied()
        .find(|&l| l >= w)
        .unwrap_or(REC_MAX_W)
}

/// 每个宽度桶的批行数:窄行多批、宽行少批,让"批 × 宽"的乘积(≈激活与输出
/// 内存)有恒定上界。3200 宽一批 8 行的输出张量就是 235MB——宽行本来稀少,
/// 批 1 足够;窄行(界面文字主体)保持 8 行大批吃满加速。
fn batch_for(ladder: u32) -> usize {
    match ladder {
        160 | 320 | 640 => 8,
        960 | 1280 => 4,
        1600 | 2240 => 2,
        _ => 1,
    }
}
/// ort 线程数(后台/常驻):消化是后台任务,不许吃满用户整机(ort 默认占所有核)。
/// 取核数的 1/4、夹在 [2,4]——32 核台机用 4 线程无感,4 核笔记本只占 2 线程。
pub(crate) fn ort_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    (cores / 4).clamp(2, 4)
}

/// ort 线程数(手动全速):用户主动点「立即回填」时希望尽快清完积压,
/// 放开到 核数-2(留两核给 UI),夹在 [4,16]——超过 16 线程单帧推理收益趋零。
pub(crate) fn ort_threads_fast() -> usize {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    cores.saturating_sub(2).clamp(4, 16)
}

/// 模型目录:`<data_root>/ai/ocr/`。
pub fn model_dir() -> PathBuf {
    crate::storage::db_path_dir()
        .map(|p| p.join("ai").join("ocr"))
        .unwrap_or_else(|_| PathBuf::from("ai").join("ocr"))
}

/// 识别出的一行(已按版面阅读序排列)。
#[derive(Debug, Clone)]
pub struct OcrLine {
    pub text: String,
}

pub struct OcrEngine {
    det: Mutex<Session>,
    rec: Mutex<Session>,
    dict: Vec<String>,
}

impl OcrEngine {
    /// 后台/常驻模式加载:保守线程数,不打扰前台使用。
    pub fn load() -> Result<Self> {
        Self::load_with_threads(ort_threads())
    }

    /// 手动全速模式加载:用户点「立即回填」时用,尽快清完积压。
    pub fn load_fast() -> Result<Self> {
        Self::load_with_threads(ort_threads_fast())
    }

    /// 从 [`model_dir`] 加载两个会话 + 字典。onnxruntime 动态库缺失时返回
    /// [`Error::EmbeddingRuntimeMissing`](统一的下载引导链路)。
    fn load_with_threads(threads: usize) -> Result<Self> {
        if !crate::ai::embedding_runtime::is_installed().unwrap_or(false) {
            return Err(Error::EmbeddingRuntimeMissing);
        }
        let dir = model_dir();
        let dict: Vec<String> = std::fs::read_to_string(dir.join("dict.txt"))
            .map_err(|e| Error::Ocr(format!("读字典失败: {e}")))?
            .lines()
            .map(str::to_string)
            .collect();
        if dict.is_empty() {
            return Err(Error::Ocr("字典为空".into()));
        }
        log::info!("OCR 引擎加载,intra threads = {threads}");
        let open = |name: &str| -> Result<Session> {
            crate::ai::onnx_session_builder(threads)
                .and_then(|b| b.commit_from_file(dir.join(name)))
                .map_err(|e| Error::Ocr(format!("加载 {name} 失败: {e}")))
        };
        Ok(Self {
            det: Mutex::new(open("det.onnx")?),
            rec: Mutex::new(open("rec.onnx")?),
            dict,
        })
    }

    /// 识别一整张图:检测行框 → 批量识别 → 按版面序(行内左到右,行间上到下)返回。
    pub fn recognize(&self, img: &DynamicImage) -> Result<Vec<OcrLine>> {
        let rgb = img.to_rgb8();
        let t_det = std::time::Instant::now();
        let boxes = self.detect(&rgb)?;
        let det_ms = t_det.elapsed().as_millis();

        // 识别单元 = 一条 48 高的行图(超长行切成多段,段序拼回)
        let t_rec = std::time::Instant::now();
        let units = prepare_units(&rgb, &boxes);
        let mut texts: Vec<String> = vec![String::new(); units.len()];
        // 按目标宽排序 → 同宽度桶的单元相邻 → 按桶的批容量分批
        let mut order: Vec<usize> = (0..units.len()).collect();
        order.sort_by_key(|&i| units[i].strip.width());
        let mut chunk: Vec<usize> = Vec::with_capacity(REC_BATCH);
        let mut chunk_ladder = 0u32;
        let flush = |chunk: &mut Vec<usize>, ladder: u32, texts: &mut Vec<String>| -> Result<()> {
            if chunk.is_empty() {
                return Ok(());
            }
            let batch: Vec<&RecUnit> = chunk.iter().map(|&i| &units[i]).collect();
            let decoded = self.rec_batch(&batch, ladder)?;
            for (&i, text) in chunk.iter().zip(decoded) {
                texts[i] = text;
            }
            chunk.clear();
            Ok(())
        };
        for &i in &order {
            let lad = ladder_w(units[i].strip.width());
            if !chunk.is_empty() && (lad != chunk_ladder || chunk.len() >= batch_for(chunk_ladder))
            {
                flush(&mut chunk, chunk_ladder, &mut texts)?;
            }
            chunk_ladder = lad;
            chunk.push(i);
        }
        flush(&mut chunk, chunk_ladder, &mut texts)?;
        // 段序拼回各框
        let mut per_box: Vec<String> = vec![String::new(); boxes.len()];
        for (u, text) in units.iter().zip(&texts) {
            per_box[u.box_idx].push_str(text);
        }
        log::debug!(
            "ocr: det {det_ms}ms + rec {}ms ({} 框 {} 单元)",
            t_rec.elapsed().as_millis(),
            boxes.len(),
            units.len()
        );
        Ok(per_box
            .into_iter()
            .filter(|t| !t.is_empty())
            .map(|text| OcrLine { text })
            .collect())
    }

    /// det:整图 → 行级文本框(原图坐标)。
    fn detect(&self, rgb: &RgbImage) -> Result<Vec<TextBox>> {
        let (ow, oh) = rgb.dimensions();
        // 长边限制 + /32 对齐(至少 32)
        let scale = (DET_LIMIT_SIDE as f32 / ow.max(oh) as f32).min(1.0);
        let tw = (((ow as f32 * scale) as u32) / 32).max(1) * 32;
        let th = (((oh as f32 * scale) as u32) / 32).max(1) * 32;
        let resized = image::imageops::resize(rgb, tw, th, image::imageops::FilterType::Triangle);

        // 张量尺寸向上取整到 /128:窗口截图尺寸各异,每种 (宽,高) 都会让
        // onnxruntime 内存池新留一块;归桶后形状种类有界,内存曲线封顶。
        // 右/下 pad 零(≈灰底),概率图上是低响应区,不产框。
        let (pw, ph) = (tw.div_ceil(128) * 128, th.div_ceil(128) * 128);

        // BGR + ImageNet mean/std(顺序与官方 yml 一致:mean[0] 作用于 B 通道)
        const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
        const STD: [f32; 3] = [0.229, 0.224, 0.225];
        let mut input = Array4::<f32>::zeros((1, 3, ph as usize, pw as usize));
        for (x, y, p) in resized.enumerate_pixels() {
            let bgr = [p[2], p[1], p[0]];
            for c in 0..3 {
                input[[0, c, y as usize, x as usize]] = (bgr[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }

        let prob: Vec<f32> = {
            let mut det = self
                .det
                .lock()
                .map_err(|e| Error::Ocr(format!("det mutex poisoned: {e}")))?;
            let tensor = TensorRef::from_array_view(input.view())
                .map_err(|e| Error::Ocr(format!("det tensor: {e}")))?;
            let outputs = det
                .run(ort::inputs![tensor])
                .map_err(|e| Error::Ocr(format!("det run: {e}")))?;
            let (shape, data) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::Ocr(format!("det output: {e}")))?;
            if shape.len() != 4 {
                return Err(Error::Ocr(format!("det 输出形状异常: {shape:?}")));
            }
            data.to_vec()
        };

        // 概率图 → 连通域 → 框(概率图按 pad 后尺寸索引);再按实际内容区映射回原图坐标
        let mut boxes = prob_map_to_boxes(&prob, pw as usize, ph as usize);
        let (rx, ry) = (ow as f32 / tw as f32, oh as f32 / th as f32);
        for b in &mut boxes {
            b.x0 = ((b.x0 as f32) * rx) as u32;
            b.x1 = (((b.x1 as f32) * rx) as u32).min(ow - 1);
            b.y0 = ((b.y0 as f32) * ry) as u32;
            b.y1 = (((b.y1 as f32) * ry) as u32).min(oh - 1);
        }
        sort_reading_order(&mut boxes);
        Ok(boxes)
    }

    /// 一批识别单元(已缩放到 48 高)→ 各自文本。张量宽 = 阶梯值、批维 =
    /// 该桶的固定批容量(不足补零行)——输入形状全有界,内存池不随长跑增长;
    /// pad 区为标准化后的 0(paddle 官方同款做法,blank 主导不产字)。
    /// 解码在锁内直接吃借用的输出张量,不复制(宽桶输出可达上百 MB)。
    fn rec_batch(&self, batch: &[&RecUnit], ladder: u32) -> Result<Vec<String>> {
        let batch_dim = batch_for(ladder);
        debug_assert!(batch.len() <= batch_dim);
        let mut input = Array4::<f32>::zeros((batch_dim, 3, REC_H as usize, ladder as usize));
        for (bi, u) in batch.iter().enumerate() {
            for (x, y, p) in u.strip.enumerate_pixels() {
                let bgr = [p[2], p[1], p[0]];
                for c in 0..3 {
                    input[[bi, c, y as usize, x as usize]] = bgr[c] as f32 / 255.0 * 2.0 - 1.0;
                }
            }
        }

        let mut rec = self
            .rec
            .lock()
            .map_err(|e| Error::Ocr(format!("rec mutex poisoned: {e}")))?;
        let tensor = TensorRef::from_array_view(input.view())
            .map_err(|e| Error::Ocr(format!("rec tensor: {e}")))?;
        let outputs = rec
            .run(ort::inputs![tensor])
            .map_err(|e| Error::Ocr(format!("rec run: {e}")))?;
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::Ocr(format!("rec output: {e}")))?;
        if shape.len() != 3 || (shape[0] as usize) < batch.len() {
            return Err(Error::Ocr(format!("rec 输出形状异常: {shape:?}")));
        }
        let (t_len, classes) = (shape[1] as usize, shape[2] as usize);
        let stride = t_len * classes;
        Ok((0..batch.len())
            .map(|bi| {
                ctc_decode(
                    &data[bi * stride..(bi + 1) * stride],
                    t_len,
                    classes,
                    &self.dict,
                )
            })
            .collect())
    }
}

/// det 出的一个文本框(含 unclip 扩边),坐标随调用方所在空间。
#[derive(Debug, Clone)]
struct TextBox {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

/// 一个识别单元:某框的第 N 段,已裁剪并缩放到 48 高的行图。
struct RecUnit {
    box_idx: usize,
    strip: RgbImage,
}

/// 把检测框变成识别单元:裁剪 → 超长行(缩放后宽 > 上限)等宽切段(2% 重叠防切字)
/// → 缩放到 48 高。单元顺序保持"框序 × 段序",拼回时天然正确。
fn prepare_units(rgb: &RgbImage, boxes: &[TextBox]) -> Vec<RecUnit> {
    let mut units = Vec::with_capacity(boxes.len());
    for (box_idx, b) in boxes.iter().enumerate() {
        let (w, h) = (b.x1 - b.x0 + 1, b.y1 - b.y0 + 1);
        if w < 4 || h < 4 {
            continue;
        }
        let crop = image::imageops::crop_imm(rgb, b.x0, b.y0, w, h).to_image();
        let target_w = (REC_H as f32 * w as f32 / h as f32) as u32;
        let n = target_w.div_ceil(REC_MAX_W).max(1);
        let seg_w = w / n;
        let overlap = (seg_w / 50).max(4);
        for i in 0..n {
            let sx = (i * seg_w).saturating_sub(if i > 0 { overlap } else { 0 });
            let sw = (seg_w + overlap).min(w - sx);
            let seg = image::imageops::crop_imm(&crop, sx, 0, sw, h).to_image();
            let sw48 = ((REC_H as f32 * sw as f32 / h as f32) as u32).clamp(16, REC_MAX_W);
            let strip =
                image::imageops::resize(&seg, sw48, REC_H, image::imageops::FilterType::Triangle);
            units.push(RecUnit { box_idx, strip });
        }
    }
    units
}

/// DB 后处理:概率图 → 二值化 → 4 邻域连通域 → 按均值分过滤 → unclip 扩框。
/// 轴对齐简化版:屏幕文字横平竖直,不做旋转四边形(照片场景才需要)。
fn prob_map_to_boxes(prob: &[f32], w: usize, h: usize) -> Vec<TextBox> {
    let mut visited = vec![false; w * h];
    let mut boxes = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for start in 0..w * h {
        if visited[start] || prob[start] < DET_THRESH {
            continue;
        }
        // flood fill 收一个连通域
        let (mut x0, mut y0, mut x1, mut y1) = (start % w, start / w, start % w, start / w);
        let (mut sum, mut count) = (0f32, 0usize);
        stack.push(start);
        visited[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
            sum += prob[i];
            count += 1;
            for n in neighbors4(x, y, w, h) {
                if !visited[n] && prob[n] >= DET_THRESH {
                    visited[n] = true;
                    stack.push(n);
                }
            }
        }
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        if bw < 3 || bh < 3 || sum / count as f32 <= DET_BOX_THRESH {
            continue;
        }
        // unclip:DB 输出的是向内收缩过的核,按面积/周长比向外扩(官方 ratio 1.5)
        let d = (bw as f32 * bh as f32 * DET_UNCLIP / (2.0 * (bw + bh) as f32)) as usize;
        boxes.push(TextBox {
            x0: x0.saturating_sub(d) as u32,
            y0: y0.saturating_sub(d) as u32,
            x1: (x1 + d).min(w - 1) as u32,
            y1: (y1 + d).min(h - 1) as u32,
        });
    }
    boxes
}

fn neighbors4(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let mut v = [usize::MAX; 4];
    if x > 0 {
        v[0] = y * w + x - 1;
    }
    if x + 1 < w {
        v[1] = y * w + x + 1;
    }
    if y > 0 {
        v[2] = (y - 1) * w + x;
    }
    if y + 1 < h {
        v[3] = (y + 1) * w + x;
    }
    v.into_iter().filter(|&i| i != usize::MAX)
}

/// 版面阅读序:按行分组(y 中心差 < 0.6×行高判同行),行间上到下、行内左到右。
fn sort_reading_order(boxes: &mut [TextBox]) {
    boxes.sort_by_key(|b| ((b.y0 + b.y1) / 2, b.x0));
    let mut i = 0;
    while i < boxes.len() {
        let row_yc = (boxes[i].y0 + boxes[i].y1) / 2;
        let row_h = boxes[i].y1 - boxes[i].y0 + 1;
        let mut j = i + 1;
        while j < boxes.len() {
            let yc = (boxes[j].y0 + boxes[j].y1) / 2;
            if yc.abs_diff(row_yc) < (row_h * 6 / 10).max(1) {
                j += 1;
            } else {
                break;
            }
        }
        boxes[i..j].sort_by_key(|b| b.x0);
        i = j;
    }
}

/// CTC 贪心解码:逐帧 argmax → 折叠连续重复 → 去 blank(0) → 查字典。
/// 类映射:1..=N → dict[i−1];N+1(存在时)→ 空格。
fn ctc_decode(data: &[f32], t_len: usize, classes: usize, dict: &[String]) -> String {
    let mut out = String::new();
    let mut prev = 0usize;
    for t in 0..t_len {
        let row = &data[t * classes..(t + 1) * classes];
        let idx = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0);
        if idx != 0 && idx != prev {
            if idx - 1 < dict.len() {
                out.push_str(&dict[idx - 1]);
            } else {
                out.push(' ');
            }
        }
        prev = idx;
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctc_collapses_repeats_and_blanks() {
        let dict: Vec<String> = ["甲", "乙", "丙"].iter().map(|s| s.to_string()).collect();
        // 类数 = 3 字典 + blank + 空格 = 5;序列: 甲 甲 _ 乙 空格 乙
        let classes = 5;
        let seq: [usize; 6] = [1, 1, 0, 2, 4, 2];
        let mut data = vec![0f32; seq.len() * classes];
        for (t, &c) in seq.iter().enumerate() {
            data[t * classes + c] = 1.0;
        }
        assert_eq!(ctc_decode(&data, seq.len(), classes, &dict), "甲乙 乙");
    }

    #[test]
    fn prob_map_two_components() {
        // 20×8 概率图:两个分离的高概率块 → 两个框(含 unclip 扩边)
        let (w, h) = (20, 8);
        let mut prob = vec![0f32; w * h];
        for y in 2..5 {
            for x in 2..8 {
                prob[y * w + x] = 0.9;
            }
            for x in 12..18 {
                prob[y * w + x] = 0.9;
            }
        }
        let boxes = prob_map_to_boxes(&prob, w, h);
        assert_eq!(boxes.len(), 2);
        assert!(boxes[0].x1 < boxes[1].x0);
    }

    /// 真模型冒烟:需要 `<data_root>/ai/ocr/` 三件套 + onnxruntime 已安装。
    /// 跑法:`OCR_TEST_IMG=<图片路径> cargo test --lib ocr -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_inference_smoke() {
        let dylib = crate::ai::embedding_runtime::dylib_path().unwrap();
        std::env::set_var("ORT_DYLIB_PATH", &dylib);
        let img_path = std::env::var("OCR_TEST_IMG").expect("设 OCR_TEST_IMG 指向测试图");
        let img = image::open(&img_path).unwrap();

        let t0 = std::time::Instant::now();
        let engine = OcrEngine::load().unwrap();
        println!("加载: {:?}", t0.elapsed());

        let t1 = std::time::Instant::now();
        let lines = engine.recognize(&img).unwrap();
        println!(
            "识别(冷,含 EP 预热): {:?} | {} 行",
            t1.elapsed(),
            lines.len()
        );
        let t2 = std::time::Instant::now();
        let _ = engine.recognize(&img).unwrap();
        println!("识别(热): {:?}", t2.elapsed());
        for l in lines.iter().take(5) {
            println!("  {}", l.text);
        }
        assert!(!lines.is_empty(), "至少识别出一行");
    }

    #[test]
    fn reading_order_rows_then_columns() {
        let mut boxes = vec![
            TextBox {
                x0: 50,
                y0: 10,
                x1: 90,
                y1: 20,
            }, // 第一行右
            TextBox {
                x0: 0,
                y0: 11,
                x1: 40,
                y1: 21,
            }, // 第一行左
            TextBox {
                x0: 0,
                y0: 40,
                x1: 40,
                y1: 50,
            }, // 第二行
        ];
        sort_reading_order(&mut boxes);
        assert_eq!(boxes[0].x0, 0);
        assert_eq!(boxes[1].x0, 50);
        assert_eq!(boxes[2].y0, 40);
    }
}
