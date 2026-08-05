//! macOS 系统 Vision framework OCR 后端。
//!
//! 跑在 Apple 神经引擎(ANE)上,功耗远低于 onnxruntime CPU 推理(M 系芯片上
//! 发热/续航差一个量级);系统自带模型,**零下载、零 onnxruntime 依赖**。
//! 质量档案(切条、字号地板、句子还原率评法)见 docs/design/screen-memory.md §L2
//! 与 scripts/poc/(当时的 POC 就是拿 Vision 测的)。
//!
//! 实现走 objc2-vision 静态绑定,进程内直调——不用 sidecar 二进制,省掉
//! 打包/公证一整套麻烦。Vision 的文本识别模型由 OS 管理:首次调用有一次性
//! 模型编译(~10s,OS 级缓存),之后 ~130ms/帧。

use std::path::Path;

use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequest};

use super::ocr::OcrLine;
use crate::error::{Error, Result};

/// 无状态引擎:Vision 的模型与缓存由系统管理,这里不持有任何资源。
pub struct VisionEngine;

impl VisionEngine {
    pub fn new() -> Self {
        Self
    }

    /// 识别一张已落盘的截图,返回版面阅读序(上到下,行内左到右)的行。
    ///
    /// **整个调用必须包在 autoreleasepool 里**,这不是内存卫生问题,是可用性问题:
    /// 调用发生在 tokio 的阻塞池线程上,那种线程没有 ambient pool,Vision 内部
    /// autorelease 的临时对象无人排空,连同它们持有的识别会话资源一起泄漏;
    /// 攒到几百次调用后,Vision 那个容量受限的 detector 队列再也拿不到名额,
    /// `performRequests` 就永久阻塞在信号量上——**整个 OCR 子系统卡死到重启为止**。
    ///
    /// 实测(4 臂浸泡,独占运行,同一张图反复识别):
    /// - 不包 pool:第 714 次卡死;
    /// - 只加这一层 pool、其余完全不变:1500 次跑完无异常。
    ///   生产两次事故分别发生在第 327 / 875 次识别之后,与实验同一量级。
    pub fn recognize_file(&self, path: &Path) -> Result<Vec<OcrLine>> {
        let p = path
            .to_str()
            .ok_or_else(|| Error::Ocr("截图路径非 UTF-8".into()))?;
        objc2::rc::autoreleasepool(|_| self.recognize_inner(p))
    }

    fn recognize_inner(&self, p: &str) -> Result<Vec<OcrLine>> {
        // SAFETY: 全部为 Vision/Foundation 公开 ObjC API;对象生命周期由
        // Retained 管理;handler/request 均为本函数局部对象,无跨线程共享。
        unsafe {
            let url = NSURL::fileURLWithPath(&NSString::from_str(p));
            let handler = VNImageRequestHandler::initWithURL_options(
                VNImageRequestHandler::alloc(),
                &url,
                &NSDictionary::new(),
            );

            let request = VNRecognizeTextRequest::new();
            request.setRecognitionLevel(objc2_vision::VNRequestTextRecognitionLevel::Accurate);
            // 自动语言检测(macOS 13+;本应用最低支持 14):混排中英日的屏幕文本
            // 显式指定语言反而更糟,POC 已验证走默认+自动检测。
            request.setAutomaticallyDetectsLanguage(true);

            // 上转型 VNRecognizeTextRequest → VNImageBasedRequest → VNRequest
            let req_base: Retained<VNRequest> =
                Retained::into_super(Retained::into_super(request.clone()));
            let requests = NSArray::from_retained_slice(&[req_base]);
            handler
                .performRequests_error(&requests)
                .map_err(|e| Error::Ocr(format!("Vision 识别失败: {e}")))?;

            let Some(results) = request.results() else {
                return Ok(Vec::new());
            };

            // Vision 的 boundingBox 是归一化坐标、原点在**左下**:
            // 阅读序 = 先按 top(1 - y - h)再按 x 排。
            let mut lines: Vec<(f64, f64, String, [f32; 4])> = Vec::new();
            for obs in results.iter() {
                let cands = obs.topCandidates(1);
                let Some(cand) = cands.firstObject() else {
                    continue;
                };
                let text = cand.string().to_string();
                if text.trim().is_empty() {
                    continue;
                }
                let bb = obs.boundingBox();
                let top = 1.0 - bb.origin.y - bb.size.height;
                lines.push((
                    top,
                    bb.origin.x,
                    text,
                    // Vision 原点在左下 → 转左上原点的归一化 [x, y, w, h]
                    [
                        bb.origin.x as f32,
                        top as f32,
                        bb.size.width as f32,
                        bb.size.height as f32,
                    ],
                ));
            }
            lines.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            Ok(lines
                .into_iter()
                .map(|(_, _, text, bx)| OcrLine {
                    text,
                    box_norm: Some(bx),
                })
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 浸泡:复刻生产调用形态(每帧一根新线程),反复识别同一张图,看会不会卡死。
    ///
    /// 这是 2026-08-04 那次事故的回归测试。当时 `recognize_file` 没包
    /// autoreleasepool,临时对象在 tokio 阻塞池线程上无人排空,几百次调用后
    /// Vision 的 detector 队列拿不到名额,`performRequests` 永久阻塞——
    /// 整个 OCR 子系统卡死到重启为止。实测无 pool 时两跑两卡(第 276、714 次),
    /// 加上 pool 后 1500 次无异常;生产两次事故分别在第 327、875 次识别之后。
    ///
    /// 单次识别超过 120 秒(正常约 0.4 秒)即判定卡死。默认 ignored:要真实截图、
    /// 耗时以分钟计。改动 Vision 调用后手动跑一遍:
    /// `HINDSIGHT_TEST_IMG=<截图> HINDSIGHT_SOAK_ITERS=3000 \
    ///    cargo test --lib vision_soak -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn vision_soak_no_wedge() {
        let Some(p) = std::env::var_os("HINDSIGHT_TEST_IMG") else {
            eprintln!("未设置 HINDSIGHT_TEST_IMG,跳过");
            return;
        };
        let iters: usize = std::env::var("HINDSIGHT_SOAK_ITERS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1500);
        let path = std::path::PathBuf::from(&p);
        let started = std::time::Instant::now();

        for i in 1..=iters {
            let path = path.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            // 每次一根全新线程即用即弃——这正是 tokio 阻塞池的行为,
            // 也是事故的必要条件(线程没有环境 pool 且很快被销毁)
            std::thread::spawn(move || {
                let r = VisionEngine::new().recognize_file(&path);
                let _ = tx.send(r.map(|l| l.len()));
            });
            match rx.recv_timeout(std::time::Duration::from_secs(120)) {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => panic!("第 {i} 次识别失败: {e}"),
                Err(_) => panic!(
                    "第 {i} 次识别超过 120 秒未返回——Vision 已卡死(事故复现),已跑 {:?}",
                    started.elapsed()
                ),
            }
            if i % 200 == 0 {
                eprintln!("  {i}/{iters} 正常(累计 {:?})", started.elapsed());
            }
        }
        eprintln!("✓ {iters} 次识别未卡死,用时 {:?}", started.elapsed());
    }

    /// 冒烟:对 HINDSIGHT_TEST_IMG 指定的真实截图跑一遍 Vision。
    /// 需要真实文件,CI 无图,故 ignored;本地验证:
    /// `HINDSIGHT_TEST_IMG=<截图路径> cargo test --lib vision_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn vision_smoke() {
        let Some(p) = std::env::var_os("HINDSIGHT_TEST_IMG") else {
            eprintln!("未设置 HINDSIGHT_TEST_IMG,跳过");
            return;
        };
        let lines = VisionEngine::new()
            .recognize_file(std::path::Path::new(&p))
            .expect("Vision 识别失败");
        let chars: usize = lines.iter().map(|l| l.text.chars().count()).sum();
        eprintln!("识别 {} 行 / {} 字符", lines.len(), chars);
        for l in lines.iter().take(5) {
            eprintln!("  · {}", l.text);
        }
        assert!(!lines.is_empty(), "真实截图应至少识别出一行");
    }
}
