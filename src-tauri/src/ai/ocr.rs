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

/// 核显档的 det 长边上限。实测(桌面 UI 6 张 + 日文论文极限小字 + B 站密集页,
/// 统一 2-gram 字符口径):1440 相对 1920 召回 ~94-95%,损失集中在 UI 角标碎字
/// (状态栏/菜单/文件树),正文与标题几乎无损——换 det 计算量 -44%,对核显本
/// 是合理权衡。独显/CPU 保 1920(算力足够,不必付这 5%)。
const DET_LIMIT_SIDE_INTEGRATED: u32 = 1440;

/// det 长边上限:按执行档位取值,可被 `HINDSIGHT_DET_SIDE` 覆盖(标定 A/B 用,
/// 生产路径不设)。
fn det_limit_side(tier: RecTier) -> u32 {
    if let Some(v) = std::env::var("HINDSIGHT_DET_SIDE")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return v;
    }
    match tier {
        RecTier::IntegratedGpu => DET_LIMIT_SIDE_INTEGRATED,
        _ => DET_LIMIT_SIDE,
    }
}
/// DB 二值化阈值 / 框置信度阈值 / 扩框比(官方 inference.yml 定案)
const DET_THRESH: f32 = 0.3;
const DET_BOX_THRESH: f32 = 0.6;
const DET_UNCLIP: f32 = 1.5;
/// rec 输入高;宽 = 高 × 纵横比,超过上限的超长行切段识别
const REC_H: u32 = 48;
const REC_MAX_W: u32 = 3200;
/// rec 张量宽的阶梯:批宽向上取整到阶梯值,让输入形状种类有界——
/// onnxruntime 的内存池按形状留块且只增不还,宽度连续变化会让内存
/// 随长跑无界增长(实测 505 帧从 1.0GB 涨到 1.4GB)。付出 ~10% padding
/// 计算,换内存曲线封顶。
const REC_W_LADDER: [u32; 8] = [160, 320, 640, 960, 1280, 1600, 2240, 3200];
/// 独显档的粗阶梯:padding 在过剩算力上免费,桶少 = 形状切换少 = 往返少。
const REC_W_LADDER_DISCRETE: [u32; 3] = [640, 1600, 3200];

/// rec 执行档位——同一套模型,按设备物理条件用不同的批/阶梯策略
/// (实测:GPU 的瓶颈是每次 run 的提交+同步固定开销 ~7ms,要大批少往返;
/// CPU/核显算力紧张,padding 是真实成本,要细阶梯贴合真宽)。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum RecTier {
    /// NVIDIA 独显 ≥2GB:批 32 粗阶梯(3 档)。AMD 独显探测不到(nvidia-smi),
    /// 落到 Integrated 档——保守方向,不会更慢只是没吃满。
    DiscreteGpu,
    /// DML 生效但无独显信号(核显/UMA):往返本就便宜些,批翻倍、阶梯保持细。
    IntegratedGpu,
    /// DML 不可用回退 CPU:现状参数(为 CPU 内存曲线与算力调校)。
    Cpu,
}

fn ladder_w(tier: RecTier, w: u32) -> u32 {
    let ladder: &[u32] = match tier {
        RecTier::DiscreteGpu => &REC_W_LADDER_DISCRETE,
        _ => &REC_W_LADDER,
    };
    ladder
        .iter()
        .copied()
        .find(|&l| l >= w)
        .unwrap_or(REC_MAX_W)
}

/// 每个宽度桶的批行数。上界考量:
/// - 输入张量 = 批×3×48×宽×4B(独显 640×32 = 11.8MB,可接受);
/// - 输出经 ArgMax 进图后是 [批,T] int64(KB 级),不再约束批大小;
///   f32 慢路径(旧模型)下批大输出也大,但慢路径本身就是过渡态。
fn batch_for(tier: RecTier, ladder: u32) -> usize {
    match tier {
        RecTier::DiscreteGpu => match ladder {
            640 => 32,
            1600 => 8,
            _ => 4,
        },
        RecTier::IntegratedGpu => match ladder {
            160 | 320 | 640 => 16,
            960 | 1280 => 8,
            1600 | 2240 => 4,
            _ => 2,
        },
        RecTier::Cpu => match ladder {
            160 | 320 | 640 => 8,
            960 | 1280 => 4,
            1600 | 2240 => 2,
            _ => 1,
        },
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
    /// 行框,归一化 [x, y, w, h](左上原点,0..1)。None = 后端未提供。
    /// 消化管线不落库;搜索页 lightbox 现场定位命中行用。
    pub box_norm: Option<[f32; 4]>,
}

/// OCR 引擎门面:macOS 默认走系统 Vision(ANE,零下载、零 onnxruntime 依赖),
/// 其它平台走 PaddleOCR ONNX。环境变量 `HINDSIGHT_OCR_PADDLE=1` 可在 macOS
/// 强制回退 Paddle(双引擎质量 A/B 与故障排查用)。
pub struct OcrEngine {
    backend: Backend,
}

enum Backend {
    #[cfg(target_os = "macos")]
    Vision(super::ocr_vision::VisionEngine),
    Paddle(PaddleEngine),
}

impl OcrEngine {
    fn use_vision() -> bool {
        cfg!(target_os = "macos") && std::env::var_os("HINDSIGHT_OCR_PADDLE").is_none()
    }

    /// 当前后端是否需要 Paddle 模型 + onnxruntime(Vision 后端不需要,
    /// 模型下载/运行时安装全部跳过)。
    pub fn needs_models() -> bool {
        !Self::use_vision()
    }

    /// 后台/常驻模式加载:保守线程数,不打扰前台使用。
    pub fn load() -> Result<Self> {
        Self::load_inner(false)
    }

    /// 手动全速模式加载:用户点「立即回填」时用,尽快清完积压。
    pub fn load_fast() -> Result<Self> {
        Self::load_inner(true)
    }

    fn load_inner(fast: bool) -> Result<Self> {
        #[cfg(target_os = "macos")]
        if Self::use_vision() {
            log::info!("OCR 引擎:系统 Vision(ANE)");
            return Ok(Self {
                backend: Backend::Vision(super::ocr_vision::VisionEngine::new()),
            });
        }
        let threads = if fast {
            ort_threads_fast()
        } else {
            ort_threads()
        };
        Ok(Self {
            backend: Backend::Paddle(PaddleEngine::load_with_threads(threads)?),
        })
    }

    /// 识别一张已落盘的截图,返回版面阅读序的行。
    /// Vision 后端直接吃文件(自带解码);Paddle 后端在此解码后走 ONNX 管线。
    pub fn recognize_file(&self, path: &std::path::Path) -> Result<Vec<OcrLine>> {
        match &self.backend {
            #[cfg(target_os = "macos")]
            Backend::Vision(v) => v.recognize_file(path),
            Backend::Paddle(p) => {
                let img = image::open(path).map_err(|e| Error::Ocr(format!("读图失败: {e}")))?;
                p.recognize(&img)
            }
        }
    }
}

/// PaddleOCR ONNX 后端(det + rec 双模型,onnxruntime CPU)。
pub(crate) struct PaddleEngine {
    det: Mutex<Session>,
    rec: Mutex<Session>,
    dict: Vec<String>,
    tier: RecTier,
}

impl PaddleEngine {
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
        let open = |name: &str| -> Result<(Session, bool)> {
            crate::ai::onnx_session_from_file(threads, &dir.join(name))
                .map_err(|e| Error::Ocr(format!("加载 {name} 失败: {e}")))
        };
        let (det, _) = open("det.onnx")?;
        // rec:读官方模型字节 → 内存 ArgMax 改图(磁盘保持原件)→ 建会话。
        // 改图失败时字节原样返回,f32 慢路径兜底(行为 = 引入优化前)。
        let rec_bytes = std::fs::read(dir.join("rec.onnx"))
            .map_err(|e| Error::Ocr(format!("读 rec.onnx 失败: {e}")))?;
        let (rec_bytes, _argmaxed) = crate::ai::ocr_patch::ensure_rec_argmax(rec_bytes);
        let (rec, rec_dml) =
            crate::ai::onnx_session_from_memory(threads, &rec_bytes, "rec.onnx+argmax")
                .map_err(|e| Error::Ocr(format!("加载 rec.onnx 失败: {e}")))?;
        let tier = if !rec_dml {
            RecTier::Cpu
        } else {
            match crate::ai::platform::detect_total_vram_gb() {
                Some(v) if v.source == "discrete" && v.total_gb >= 2.0 => RecTier::DiscreteGpu,
                _ => RecTier::IntegratedGpu,
            }
        };
        // HINDSIGHT_REC_TIER=discrete|integrated|cpu 覆盖档位:质量/性能 A/B 标定用
        let tier = match std::env::var("HINDSIGHT_REC_TIER").as_deref() {
            Ok("discrete") => RecTier::DiscreteGpu,
            Ok("integrated") => RecTier::IntegratedGpu,
            Ok("cpu") => RecTier::Cpu,
            _ => tier,
        };
        log::info!("rec 执行档位: {tier:?}");
        let engine = Self {
            det: Mutex::new(det),
            rec: Mutex::new(rec),
            dict,
            tier,
        };
        // 预热:最小 dummy 推理把 DML 图编译/内存池分配挪到加载期,
        // 首帧不再额外慢几百 ms。失败只 warn(真实推理路径自带错误处理)。
        let t = std::time::Instant::now();
        if let Err(e) = engine.warmup() {
            log::warn!("OCR 预热失败(不影响使用): {e}");
        } else {
            log::debug!("OCR 预热完成: {}ms", t.elapsed().as_millis());
        }
        Ok(engine)
    }

    /// 最小 dummy 推理各跑一次 det/rec(预热用)。
    fn warmup(&self) -> Result<()> {
        let det_in = Array4::<f32>::zeros((1, 3, 128, 128));
        {
            let mut det = self
                .det
                .lock()
                .map_err(|e| Error::Ocr(format!("det mutex poisoned: {e}")))?;
            let t = TensorRef::from_array_view(det_in.view())
                .map_err(|e| Error::Ocr(format!("warmup det tensor: {e}")))?;
            det.run(ort::inputs![t])
                .map_err(|e| Error::Ocr(format!("warmup det: {e}")))?;
        }
        let rec_in = Array4::<f32>::zeros((1, 3, REC_H as usize, 160));
        let mut rec = self
            .rec
            .lock()
            .map_err(|e| Error::Ocr(format!("rec mutex poisoned: {e}")))?;
        let t = TensorRef::from_array_view(rec_in.view())
            .map_err(|e| Error::Ocr(format!("warmup rec tensor: {e}")))?;
        rec.run(ort::inputs![t])
            .map_err(|e| Error::Ocr(format!("warmup rec: {e}")))?;
        Ok(())
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
        let crop_ms = t_rec.elapsed().as_millis();
        let mut tm = RecTiming::default();
        let mut texts: Vec<String> = vec![String::new(); units.len()];
        // 按目标宽排序 → 同宽度桶的单元相邻 → 按桶的批容量分批
        let mut order: Vec<usize> = (0..units.len()).collect();
        order.sort_by_key(|&i| units[i].strip.width());
        let mut chunk: Vec<usize> = Vec::with_capacity(32);
        let mut chunk_ladder = 0u32;
        let flush = |chunk: &mut Vec<usize>,
                     ladder: u32,
                     texts: &mut Vec<String>,
                     tm: &mut RecTiming|
         -> Result<()> {
            if chunk.is_empty() {
                return Ok(());
            }
            let batch: Vec<&RecUnit> = chunk.iter().map(|&i| &units[i]).collect();
            let decoded = self.rec_batch(&batch, ladder, tm)?;
            for (&i, text) in chunk.iter().zip(decoded) {
                texts[i] = text;
            }
            chunk.clear();
            Ok(())
        };
        for &i in &order {
            let lad = ladder_w(self.tier, units[i].strip.width());
            if !chunk.is_empty()
                && (lad != chunk_ladder || chunk.len() >= batch_for(self.tier, chunk_ladder))
            {
                flush(&mut chunk, chunk_ladder, &mut texts, &mut tm)?;
            }
            chunk_ladder = lad;
            chunk.push(i);
        }
        flush(&mut chunk, chunk_ladder, &mut texts, &mut tm)?;
        // 段序拼回各框
        let mut per_box: Vec<String> = vec![String::new(); boxes.len()];
        for (u, text) in units.iter().zip(&texts) {
            per_box[u.box_idx].push_str(text);
        }
        log::debug!(
            "ocr: det {det_ms}ms + rec {}ms [crop {crop_ms} + norm {} + infer {} + ctc {}] ({} 框 {} 单元)",
            t_rec.elapsed().as_millis(),
            tm.norm.as_millis(),
            tm.infer.as_millis(),
            tm.ctc.as_millis(),
            boxes.len(),
            units.len()
        );
        let (iw, ih) = (rgb.width() as f32, rgb.height() as f32);
        Ok(per_box
            .into_iter()
            .zip(&boxes)
            .filter(|(t, _)| !t.is_empty())
            .map(|(text, b)| OcrLine {
                text,
                box_norm: Some([
                    b.x0 as f32 / iw,
                    b.y0 as f32 / ih,
                    (b.x1 - b.x0) as f32 / iw,
                    (b.y1 - b.y0) as f32 / ih,
                ]),
            })
            .collect())
    }

    /// det:整图 → 行级文本框(原图坐标)。
    fn detect(&self, rgb: &RgbImage) -> Result<Vec<TextBox>> {
        let (ow, oh) = rgb.dimensions();
        // 长边限制 + /32 对齐(至少 32)
        let scale = (det_limit_side(self.tier) as f32 / ow.max(oh) as f32).min(1.0);
        let tw = (((ow as f32 * scale) as u32) / 32).max(1) * 32;
        let th = (((oh as f32 * scale) as u32) / 32).max(1) * 32;
        let t_resize = std::time::Instant::now();
        let resized = simd_resize_rgb(rgb, tw, th)?;
        let resize_ms = t_resize.elapsed().as_millis();

        // 张量尺寸向上取整到 /128:窗口截图尺寸各异,每种 (宽,高) 都会让
        // onnxruntime 内存池新留一块;归桶后形状种类有界,内存曲线封顶。
        // 右/下 pad 零(≈灰底),概率图上是低响应区,不产框。
        let (pw, ph) = (tw.div_ceil(128) * 128, th.div_ceil(128) * 128);

        // BGR + ImageNet mean/std(顺序与官方 yml 一致:mean[0] 作用于 B 通道)
        const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
        const STD: [f32; 3] = [0.229, 0.224, 0.225];
        let t_norm = std::time::Instant::now();
        let mut input = Array4::<f32>::zeros((1, 3, ph as usize, pw as usize));
        for (x, y, p) in resized.enumerate_pixels() {
            let bgr = [p[2], p[1], p[0]];
            for c in 0..3 {
                input[[0, c, y as usize, x as usize]] = (bgr[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
        let norm_ms = t_norm.elapsed().as_millis();

        let t_infer = std::time::Instant::now();
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

        let infer_ms = t_infer.elapsed().as_millis();

        // 概率图 → 连通域 → 框(概率图按 pad 后尺寸索引);再按实际内容区映射回原图坐标
        let t_post = std::time::Instant::now();
        let mut boxes = prob_map_to_boxes(&prob, pw as usize, ph as usize);
        log::debug!(
            "det 分段: resize {resize_ms}ms + norm {norm_ms}ms + infer {infer_ms}ms + post {}ms",
            t_post.elapsed().as_millis()
        );
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
    fn rec_batch(
        &self,
        batch: &[&RecUnit],
        ladder: u32,
        tm: &mut RecTiming,
    ) -> Result<Vec<String>> {
        let batch_dim = batch_for(self.tier, ladder);
        debug_assert!(batch.len() <= batch_dim);
        let t_norm = std::time::Instant::now();
        let mut input = Array4::<f32>::zeros((batch_dim, 3, REC_H as usize, ladder as usize));
        for (bi, u) in batch.iter().enumerate() {
            for (x, y, p) in u.strip.enumerate_pixels() {
                let bgr = [p[2], p[1], p[0]];
                for c in 0..3 {
                    input[[bi, c, y as usize, x as usize]] = bgr[c] as f32 / 255.0 * 2.0 - 1.0;
                }
            }
        }
        tm.norm += t_norm.elapsed();

        let t_infer = std::time::Instant::now();
        let mut rec = self
            .rec
            .lock()
            .map_err(|e| Error::Ocr(format!("rec mutex poisoned: {e}")))?;
        let tensor = TensorRef::from_array_view(input.view())
            .map_err(|e| Error::Ocr(format!("rec tensor: {e}")))?;
        let outputs = rec
            .run(ort::inputs![tensor])
            .map_err(|e| Error::Ocr(format!("rec run: {e}")))?;

        // 双路径:ArgMax 已进图的模型输出 [N,T] int64 索引(快路径——argmax 由
        // GPU/优化 kernel 完成,GPU→CPU 拷贝从 ~190MB/批 缩到 KB 级);
        // 原始模型输出 [N,T,C] f32 概率(慢路径,CPU 侧标量 argmax)。
        // 按输出 dtype 动态分支,新旧模型文件都能跑,分发升级不断链。
        if let Ok((shape, idx)) = outputs[0].try_extract_tensor::<i64>() {
            if shape.len() != 2 || (shape[0] as usize) < batch.len() {
                return Err(Error::Ocr(format!("rec argmax 输出形状异常: {shape:?}")));
            }
            tm.infer += t_infer.elapsed();
            let t_len = shape[1] as usize;
            let t_ctc = std::time::Instant::now();
            let out: Vec<String> = (0..batch.len())
                .map(|bi| ctc_collapse(&idx[bi * t_len..(bi + 1) * t_len], &self.dict))
                .collect();
            tm.ctc += t_ctc.elapsed();
            return Ok(out);
        }
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::Ocr(format!("rec output: {e}")))?;
        if shape.len() != 3 || (shape[0] as usize) < batch.len() {
            return Err(Error::Ocr(format!("rec 输出形状异常: {shape:?}")));
        }
        tm.infer += t_infer.elapsed();
        let (t_len, classes) = (shape[1] as usize, shape[2] as usize);
        let stride = t_len * classes;
        let t_ctc = std::time::Instant::now();
        let out: Vec<String> = (0..batch.len())
            .map(|bi| {
                ctc_decode(
                    &data[bi * stride..(bi + 1) * stride],
                    t_len,
                    classes,
                    &self.dict,
                )
            })
            .collect();
        tm.ctc += t_ctc.elapsed();
        Ok(out)
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

/// rec 段分项耗时聚合(帧级):归一化 / 推理(含输出提取) / CTC 解码。
#[derive(Default)]
struct RecTiming {
    norm: std::time::Duration,
    infer: std::time::Duration,
    ctc: std::time::Duration,
}

/// 一个识别单元:某框的第 N 段,已裁剪并缩放到 48 高的行图。
struct RecUnit {
    box_idx: usize,
    strip: RgbImage,
}

/// SIMD 缩放(fast_image_resize,Bilinear ≈ image 的 Triangle):det 整帧
/// 2560→1920 实测比 image::imageops::resize 快 3-4 倍,是 det 段预处理的大头。
fn simd_resize_rgb(rgb: &RgbImage, tw: u32, th: u32) -> Result<RgbImage> {
    use fast_image_resize as fr;
    let src = fr::images::Image::from_vec_u8(
        rgb.width(),
        rgb.height(),
        rgb.as_raw().clone(),
        fr::PixelType::U8x3,
    )
    .map_err(|e| Error::Ocr(format!("resize src: {e}")))?;
    let mut dst = fr::images::Image::new(tw, th, fr::PixelType::U8x3);
    fr::Resizer::new()
        .resize(
            &src,
            &mut dst,
            &fr::ResizeOptions::new()
                .resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Bilinear)),
        )
        .map_err(|e| Error::Ocr(format!("resize: {e}")))?;
    RgbImage::from_raw(tw, th, dst.into_vec())
        .ok_or_else(|| Error::Ocr("resize 输出尺寸不符".into()))
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
            let strip = simd_resize_rgb(&seg, sw48, REC_H).unwrap_or_else(|_| {
                image::imageops::resize(&seg, sw48, REC_H, image::imageops::FilterType::Triangle)
            });
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
/// CTC 折叠(索引序列版):重复合并 + blank(0) 跳过。ArgMax 进图后的快路径。
/// 类映射与 [`ctc_decode`] 一致:idx-1 → dict,越界 → 空格。
fn ctc_collapse(indices: &[i64], dict: &[String]) -> String {
    let mut out = String::new();
    let mut prev = 0i64;
    for &idx in indices {
        if idx != 0 && idx != prev {
            let di = (idx - 1) as usize;
            if di < dict.len() {
                out.push_str(&dict[di]);
            } else {
                out.push(' ');
            }
        }
        prev = idx;
    }
    out.trim().to_string()
}

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
        // 定点测 Paddle 后端(macOS 上门面默认给 Vision,这里绕开门面直取)
        let engine = PaddleEngine::load_with_threads(ort_threads()).unwrap();
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
    fn ctc_collapse_matches_f32_path() {
        let dict: Vec<String> = ["甲", "乙", "丙"].iter().map(|s| s.to_string()).collect();
        // 与 ctc_collapses_repeats_and_blanks 同一序列,两条路径结果必须一致
        let seq: [i64; 6] = [1, 1, 0, 2, 4, 2];
        assert_eq!(ctc_collapse(&seq, &dict), "甲乙 乙");
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

    /// DML vs CPU 实测对照:同一模型同一输入,各 warmup 后计时,验证 DirectML
    /// EP 是否真的生效、生效后是否更快(PP-OCR 是动态形状模型,DML 有已知的
    /// 重编译开销风险)。
    /// 跑法(要求生产安装目录已有 DML 构建的 runtime 与模型):
    /// `cargo test --release --lib dml_vs_cpu_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dml_vs_cpu_bench() {
        use ort::value::TensorRef;
        std::env::set_var(
            "ORT_DYLIB_PATH",
            crate::ai::embedding_runtime::dylib_path().unwrap(),
        );
        let dir = model_dir();

        let build = |dml: bool, name: &str| -> ort::session::Session {
            let mut b = ort::session::Session::builder()
                .unwrap()
                .with_intra_threads(4)
                .unwrap()
                .with_memory_pattern(false)
                .unwrap();
            if dml {
                b = b
                    .with_execution_providers([
                        ort::execution_providers::DirectMLExecutionProvider::default()
                            .build()
                            .error_on_failure(),
                    ])
                    .expect("DML EP 注册失败(runtime 不带 DML?)");
            }
            b.commit_from_file(dir.join(name)).unwrap()
        };

        let bench =
            |sess: &mut ort::session::Session, shape: (usize, usize, usize, usize), label: &str| {
                let input = ndarray::Array4::<f32>::from_elem(shape, 0.5f32);
                // warmup 2 次(DML 首跑含图编译)
                for _ in 0..2 {
                    let t = TensorRef::from_array_view(input.view()).unwrap();
                    let _ = sess.run(ort::inputs![t]).unwrap();
                }
                let n = 5;
                let t0 = std::time::Instant::now();
                for _ in 0..n {
                    let t = TensorRef::from_array_view(input.view()).unwrap();
                    let _ = sess.run(ort::inputs![t]).unwrap();
                }
                eprintln!(
                    "{label}: {:.1} ms/次",
                    t0.elapsed().as_millis() as f64 / n as f64
                );
            };

        // det:1920 档(实际存档缩放后的典型输入)
        let mut det_cpu = build(false, "det.onnx");
        let mut det_dml = build(true, "det.onnx");
        bench(&mut det_cpu, (1, 3, 1088, 1920), "det 1920x1088 CPU");
        bench(&mut det_dml, (1, 3, 1088, 1920), "det 1920x1088 DML");

        // rec:批 8、宽 320(界面文字主体桶)
        let mut rec_cpu = build(false, "rec.onnx");
        let mut rec_dml = build(true, "rec.onnx");
        bench(&mut rec_cpu, (8, 3, 48, 320), "rec 8x48x320  CPU");
        bench(&mut rec_dml, (8, 3, 48, 320), "rec 8x48x320  DML");
        // 换一个宽度桶再测:动态形状下 DML 每个新形状要重编译,
        // warmup 已吸收;这里看稳态
        bench(&mut rec_cpu, (4, 3, 48, 960), "rec 4x48x960  CPU");
        bench(&mut rec_dml, (4, 3, 48, 960), "rec 4x48x960  DML");
    }

    /// 速度提升回归证明:同机自对照,三项优化各自与"旧路径"跑同一张真实
    /// 截图,断言新路径更快且(ArgMax 项)输出逐行一致。旧路径都还在代码里:
    /// - f32 慢路径 = 不做 ArgMax 改图的原始官方模型(双路径解码天然支持)
    /// - 旧批策略 = RecTier::Cpu 的批 8/细阶梯(优化前的参数)跑在同一 GPU 上
    /// - 旧缩放 = image::imageops::resize(Triangle)
    ///
    /// 跑法:`cargo test --release --lib speedup_regression_proof -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn speedup_regression_proof() {
        std::env::set_var(
            "ORT_DYLIB_PATH",
            crate::ai::embedding_runtime::dylib_path().unwrap(),
        );
        let img_path = std::env::var_os("OCR_BENCH_IMG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let shots = crate::storage::db_path_dir().unwrap().join("screenshots");
                let mut latest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
                for day in std::fs::read_dir(&shots).expect("screenshots 目录不存在") {
                    let day = day.unwrap().path();
                    if !day.is_dir() {
                        continue;
                    }
                    for f in std::fs::read_dir(&day).unwrap() {
                        let f = f.unwrap();
                        let path = f.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("jpg") {
                            continue;
                        }
                        let m = f.metadata().unwrap().modified().unwrap();
                        if latest.as_ref().is_none_or(|(t, _)| m > *t) {
                            latest = Some((m, path));
                        }
                    }
                }
                latest.expect("没找到任何截图").1
            });
        let img = image::open(&img_path).unwrap();
        eprintln!(
            "测试图: {}({}x{})",
            img_path.display(),
            img.width(),
            img.height()
        );

        let dir = model_dir();
        let dict: Vec<String> = std::fs::read_to_string(dir.join("dict.txt"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        let raw = std::fs::read(dir.join("rec.onnx")).unwrap();
        let (patched, ok) = crate::ai::ocr_patch::ensure_rec_argmax(raw.clone());
        assert!(ok, "改图失败,无从对照");

        // 手工构造引擎:固定 tier,只变要对照的那一项
        let build = |rec_bytes: &[u8], tier: RecTier| -> PaddleEngine {
            let (det, _) =
                crate::ai::onnx_session_from_file(ort_threads_fast(), &dir.join("det.onnx"))
                    .unwrap();
            let (rec, _) =
                crate::ai::onnx_session_from_memory(ort_threads_fast(), rec_bytes, "bench")
                    .unwrap();
            PaddleEngine {
                det: Mutex::new(det),
                rec: Mutex::new(rec),
                dict: dict.clone(),
                tier,
            }
        };
        let time_of = |eng: &PaddleEngine| -> (f64, Vec<String>) {
            let _ = eng.recognize(&img).unwrap(); // warmup
            let n = 3;
            let t0 = std::time::Instant::now();
            let mut lines = Vec::new();
            for _ in 0..n {
                lines = eng.recognize(&img).unwrap();
            }
            (
                t0.elapsed().as_millis() as f64 / n as f64,
                lines.into_iter().map(|l| l.text).collect(),
            )
        };

        // ── 对照 1:ArgMax 进图 vs f32 慢路径(同 tier 同 GPU,只变模型形态) ──
        let eng_f32 = build(&raw, RecTier::DiscreteGpu);
        let eng_arg = build(&patched, RecTier::DiscreteGpu);
        let (t_f32, lines_f32) = time_of(&eng_f32);
        let (t_arg, lines_arg) = time_of(&eng_arg);
        eprintln!(
            "[1] f32 慢路径 {t_f32:.0}ms vs ArgMax {t_arg:.0}ms(-{:.0}%)",
            (1.0 - t_arg / t_f32) * 100.0
        );
        assert_eq!(
            lines_f32, lines_arg,
            "ArgMax 必须与 f32 输出逐行一致(数学等价)"
        );
        assert!(
            t_arg < t_f32 * 0.95,
            "ArgMax 路径未见提速: {t_arg} vs {t_f32}"
        );

        // ── 对照 2:独显批策略(批32/粗阶梯) vs 旧参数(批8/细阶梯,同 GPU) ──
        let eng_old_batch = build(&patched, RecTier::Cpu); // Cpu 档参数 = 优化前的批策略
        let (t_old, _) = time_of(&eng_old_batch);
        eprintln!(
            "[2] 旧批策略 {t_old:.0}ms vs 独显批策略 {t_arg:.0}ms(-{:.0}%)",
            (1.0 - t_arg / t_old) * 100.0
        );
        assert!(
            t_arg < t_old * 0.9,
            "独显批策略未见提速: {t_arg} vs {t_old}"
        );

        // ── 对照 3:SIMD 缩放 vs image::imageops::resize ──
        let rgb = img.to_rgb8();
        let (tw, th) = (1920u32, 1080u32);
        let n = 5;
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            let _ = image::imageops::resize(&rgb, tw, th, image::imageops::FilterType::Triangle);
        }
        let t_img = t0.elapsed().as_millis() as f64 / n as f64;
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            let _ = simd_resize_rgb(&rgb, tw, th).unwrap();
        }
        let t_simd = t0.elapsed().as_millis() as f64 / n as f64;
        eprintln!(
            "[3] image::resize {t_img:.1}ms vs SIMD {t_simd:.1}ms({:.1}x)",
            t_img / t_simd
        );
        assert!(t_simd < t_img, "SIMD 缩放未见提速: {t_simd} vs {t_img}");

        eprintln!("==== 三项对照全部通过:提速为真 ====");
    }

    /// det 1920 vs 1440 质量 A/B:取最近 N 张真实截图,比较有效行(≥6 字符)
    /// 召回。rec 裁剪自原图,det 分辨率只影响"框有没有检出"——所以口径就是
    /// 行召回。跑法:
    /// `cargo test --release --lib det_side_quality_ab -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn det_side_quality_ab() {
        std::env::set_var(
            "ORT_DYLIB_PATH",
            crate::ai::embedding_runtime::dylib_path().unwrap(),
        );
        // 最近 N 张截图(按修改时间倒序)
        let shots = crate::storage::db_path_dir().unwrap().join("screenshots");
        let mut all: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
        for day in std::fs::read_dir(&shots).expect("screenshots 目录不存在") {
            let day = day.unwrap().path();
            if !day.is_dir() {
                continue;
            }
            for f in std::fs::read_dir(&day).unwrap() {
                let f = f.unwrap();
                let path = f.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jpg") {
                    all.push((f.metadata().unwrap().modified().unwrap(), path));
                }
            }
        }
        all.sort_by_key(|b| std::cmp::Reverse(b.0));
        let n = 6.min(all.len());
        assert!(n > 0, "没有截图可测");

        let valid = |lines: &[OcrLine]| -> std::collections::BTreeSet<String> {
            lines
                .iter()
                .map(|l| l.text.trim().to_string())
                .filter(|t| t.chars().count() >= 6)
                .collect()
        };

        // 归一化 Levenshtein 相似度(短行足够快)
        fn sim(a: &str, b: &str) -> f64 {
            let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
            let (la, lb) = (a.len(), b.len());
            if la == 0 || lb == 0 {
                return 0.0;
            }
            let mut prev: Vec<usize> = (0..=lb).collect();
            let mut cur = vec![0usize; lb + 1];
            for i in 1..=la {
                cur[0] = i;
                for j in 1..=lb {
                    let cost = usize::from(a[i - 1] != b[j - 1]);
                    cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
                }
                std::mem::swap(&mut prev, &mut cur);
            }
            1.0 - prev[lb] as f64 / la.max(lb) as f64
        }

        let eng = PaddleEngine::load_with_threads(ort_threads_fast()).unwrap();
        for side in ["1440", "1080"] {
            let mut tot_base = 0usize;
            let mut tot_exact = 0usize;
            let mut tot_near = 0usize;
            let mut gram_hit = 0usize;
            let mut gram_total = 0usize;
            for (_, path) in all.iter().take(n) {
                let img = image::open(path).unwrap();
                std::env::remove_var("HINDSIGHT_DET_SIDE");
                let base = valid(&eng.recognize(&img).unwrap());
                std::env::set_var("HINDSIGHT_DET_SIDE", side);
                let low = valid(&eng.recognize(&img).unwrap());
                std::env::remove_var("HINDSIGHT_DET_SIDE");
                // 2-gram 覆盖(顺序无关,字符级)
                let bigrams = |set: &std::collections::BTreeSet<String>| {
                    let joined: Vec<char> = set.iter().flat_map(|s| s.chars()).collect();
                    let mut m = std::collections::HashMap::<(char, char), usize>::new();
                    for w in joined.windows(2) {
                        *m.entry((w[0], w[1])).or_default() += 1;
                    }
                    m
                };
                let (bt, bl) = (bigrams(&base), bigrams(&low));
                gram_total += bt.values().sum::<usize>();
                gram_hit += bt
                    .iter()
                    .map(|(g, c)| (*c).min(bl.get(g).copied().unwrap_or(0)))
                    .sum::<usize>();
                let low_vec: Vec<&String> = low.iter().collect();
                let mut exact = 0;
                let mut near = 0;
                let mut misses: Vec<&String> = Vec::new();
                for b in &base {
                    if low.contains(b) {
                        exact += 1;
                    } else if low_vec.iter().any(|l| sim(b, l) >= 0.6) {
                        near += 1;
                    } else {
                        misses.push(b);
                    }
                }
                tot_base += base.len();
                tot_exact += exact;
                tot_near += near;
                eprintln!(
                    "{} @{side}: 基准 {},exact {},近似 {},真丢 {}",
                    path.file_name().unwrap().to_string_lossy(),
                    base.len(),
                    exact,
                    near,
                    misses.len()
                );
                for miss in misses.iter().take(4) {
                    eprintln!("   真丢: {miss}");
                }
            }
            eprintln!(
                "==== @{side} 汇总: 基准 {tot_base} 行,exact {:.1}%,宽松召回(exact+近似) {:.1}%",
                tot_exact as f64 / tot_base.max(1) as f64 * 100.0,
                (tot_exact + tot_near) as f64 / tot_base.max(1) as f64 * 100.0
            );
            eprintln!(
                "==== @{side} 2-gram 字符召回(以 1920 输出为参考): {:.1}%",
                gram_hit as f64 / gram_total.max(1) as f64 * 100.0
            );
        }
    }

    /// 端到端单帧耗时:真实截图,CPU 全程 vs GPU 全程,decode 与推理分段。
    /// 图片:环境变量 OCR_BENCH_IMG 指定,否则自动挑 screenshots 下最新 jpg。
    /// 跑法:`cargo test --release --lib ocr_frame_e2e_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ocr_frame_e2e_bench() {
        std::env::set_var(
            "ORT_DYLIB_PATH",
            crate::ai::embedding_runtime::dylib_path().unwrap(),
        );
        // 找一张真实截图
        let img_path = std::env::var_os("OCR_BENCH_IMG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let shots = crate::storage::db_path_dir().unwrap().join("screenshots");
                let mut latest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
                for day in std::fs::read_dir(&shots).expect("screenshots 目录不存在") {
                    let day = day.unwrap().path();
                    if !day.is_dir() {
                        continue;
                    }
                    for f in std::fs::read_dir(&day).unwrap() {
                        let f = f.unwrap();
                        let path = f.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("jpg") {
                            continue;
                        }
                        let m = f.metadata().unwrap().modified().unwrap();
                        if latest.as_ref().is_none_or(|(t, _)| m > *t) {
                            latest = Some((m, path));
                        }
                    }
                }
                latest.expect("没找到任何截图").1
            });
        let meta = std::fs::metadata(&img_path).unwrap();
        eprintln!(
            "测试图: {}({:.0} KB)",
            img_path.display(),
            meta.len() as f64 / 1024.0
        );

        // decode 单独计时(两路共同成本)
        let t = std::time::Instant::now();
        let img = image::open(&img_path).unwrap();
        eprintln!(
            "decode: {:.0} ms({}x{})",
            t.elapsed().as_millis(),
            img.width(),
            img.height()
        );

        let run = |label: &str| -> Vec<String> {
            let eng = PaddleEngine::load_with_threads(ort_threads_fast()).unwrap();
            // warmup 1 次(DML 图编译 / CPU 缓存)
            let _ = eng.recognize(&img).unwrap();
            let n = 3;
            let t0 = std::time::Instant::now();
            let mut lines = Vec::new();
            for _ in 0..n {
                lines = eng.recognize(&img).unwrap();
            }
            eprintln!(
                "{label}: {:.0} ms/帧(不含 decode,{} 行)",
                t0.elapsed().as_millis() as f64 / n as f64,
                lines.len()
            );
            lines.into_iter().map(|l| l.text).collect()
        };

        // debug 级日志让 recognize 内部的 "det Xms + rec Yms" 分段可见
        let _ = env_logger::builder()
            .filter_module("hindsight_lib::ai::ocr", log::LevelFilter::Debug)
            .is_test(false)
            .try_init();
        std::env::set_var("HINDSIGHT_OCR_CPU", "1");
        let cpu_lines = run("CPU 全程");
        std::env::remove_var("HINDSIGHT_OCR_CPU");
        let gpu_lines = run("GPU 全程");
        // 两档文本落盘供质量 diff(阶梯不同 → padding 不同,输出可能有边缘差异)
        if let Ok(dir) = std::env::var("OCR_BENCH_DUMP") {
            std::fs::write(
                format!("{dir}/cpu_lines.txt"),
                cpu_lines.join(
                    "
",
                ),
            )
            .unwrap();
            std::fs::write(
                format!("{dir}/gpu_lines.txt"),
                gpu_lines.join(
                    "
",
                ),
            )
            .unwrap();
            eprintln!("已 dump 到 {dir}");
        }
    }
}
