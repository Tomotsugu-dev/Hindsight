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
    pub fn recognize_file(&self, path: &Path) -> Result<Vec<OcrLine>> {
        let p = path
            .to_str()
            .ok_or_else(|| Error::Ocr("截图路径非 UTF-8".into()))?;
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
            let mut lines: Vec<(f64, f64, String)> = Vec::new();
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
                lines.push((1.0 - bb.origin.y - bb.size.height, bb.origin.x, text));
            }
            lines.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            Ok(lines
                .into_iter()
                .map(|(_, _, text)| OcrLine { text })
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
