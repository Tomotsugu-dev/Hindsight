//! OCR worker 子进程的线协议:一行一条 JSON(NDJSON),父进程写 stdin、
//! worker 回 stdout,同一时刻至多一个请求在飞。
//!
//! 为什么是子进程 + 最笨的管道协议(2026-08-05 定案):识别引擎会挂
//! ——macOS Vision 在进程内累积后死锁(实测多轮浸泡,进程内任何手段都救不回,
//! 见 `ocr_vision.rs` 的浸泡测试),Windows ORT/DirectML 亦有同类停摆。
//! 进程内的调用**取消不掉**,只有进程边界能把挂死清理干净:杀掉 worker,
//! 线程、ANE/GPU 句柄由操作系统回收,重启一个继续。截图本就在盘上,
//! 传路径即可;stdin EOF 即父进程已死,worker 自行退出——这条同时补上了
//! macOS 上 job_guard 防不住孤儿的缺口。
//!
//! 鲁棒性约定:worker 的 stdout 里**只有**协议行,但 ORT/CoreML 等原生库
//! 可能污染 stdout——父进程对解析不了的行只记日志、跳过,绝不因此判请求失败。
//! 字段全部 Option + 分类方法,不用 untagged enum:内部协议,稳最重要。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::ocr::OcrLine;

/// 协议版本。握手时比对:版本不合立即判 worker 不可用——
/// 覆盖"app 原地升级后,老父进程 re-exec 出新二进制"的窗口。
pub const PROTOCOL_V: u32 = 1;

/// 单行最大字节数。4K 密排 CJK 一帧的 JSON 约 200KB,8MiB 给 40 倍余量;
/// 超限说明 worker 疯了(或 stdout 被大段污染),杀掉重来比撑爆内存强。
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// 父 → worker 的一行请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireReq {
    pub v: u32,
    pub id: u64,
    pub op: ReqOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReqOp {
    Recognize,
    Shutdown,
}

impl WireReq {
    pub fn recognize(id: u64, path: PathBuf) -> Self {
        Self {
            v: PROTOCOL_V,
            id,
            op: ReqOp::Recognize,
            path: Some(path),
        }
    }

    pub fn shutdown(id: u64) -> Self {
        Self {
            v: PROTOCOL_V,
            id,
            op: ReqOp::Shutdown,
            path: None,
        }
    }
}

/// worker → 父的一行消息。三种形态共用一个结构,靠 [`WireMsg::classify`] 区分:
/// 内部协议不追求类型花活,追求"任何一行都能被无歧义地解析或丢弃"。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireMsg {
    #[serde(default)]
    pub v: u32,
    /// "ready" / "fatal";识别结果行不带 op
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<OcrLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

/// worker 回报的错误码。全部是**帧级**错误(经由健康管道回来的);
/// 设施级错误(超时/进程死/协议违规)由父进程自己合成,不走线上。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrCode {
    /// 确定可归责这张图的问题:读不出来(损坏/半写入/0 尺寸),或内容令
    /// 检测器误检爆炸(det 框数熔断,见 ocr.rs 的 DET_MAX_BOXES)
    Decode,
    /// 引擎推理失败(这一张的问题)
    Engine,
    /// 请求本身不合法(缺 path 等)
    BadRequest,
    /// Paddle 模型文件缺失
    ModelsMissing,
    /// onnxruntime 未安装
    RuntimeMissing,
    /// 未知码(前向兼容:新 worker 配老父进程时不崩)
    #[serde(other)]
    Unknown,
}

/// [`WireMsg`] 的语义形态。
#[derive(Debug)]
pub enum MsgKind {
    Ready {
        backend: String,
        pid: u32,
    },
    Fatal {
        code: ErrCode,
        msg: String,
    },
    Result {
        id: u64,
        ok: bool,
    },
    /// 结构上是 JSON 但语义不完整——按污染处理(跳过)
    Garbage,
}

impl WireMsg {
    pub fn ready(backend: &str) -> Self {
        Self {
            v: PROTOCOL_V,
            op: Some("ready".into()),
            backend: Some(backend.into()),
            pid: Some(std::process::id()),
            ..Default::default()
        }
    }

    pub fn fatal(code: ErrCode, msg: String) -> Self {
        Self {
            v: PROTOCOL_V,
            op: Some("fatal".into()),
            code: Some(code),
            msg: Some(msg),
            ..Default::default()
        }
    }

    pub fn result_ok(id: u64, lines: Vec<OcrLine>) -> Self {
        Self {
            v: PROTOCOL_V,
            id: Some(id),
            ok: Some(true),
            lines,
            ..Default::default()
        }
    }

    pub fn result_err(id: u64, code: ErrCode, msg: String) -> Self {
        Self {
            v: PROTOCOL_V,
            id: Some(id),
            ok: Some(false),
            code: Some(code),
            msg: Some(msg),
            ..Default::default()
        }
    }

    pub fn classify(&self) -> MsgKind {
        match (self.op.as_deref(), self.id, self.ok) {
            (Some("ready"), _, _) => MsgKind::Ready {
                backend: self.backend.clone().unwrap_or_default(),
                pid: self.pid.unwrap_or(0),
            },
            (Some("fatal"), _, _) => MsgKind::Fatal {
                code: self.code.unwrap_or(ErrCode::Unknown),
                msg: self.msg.clone().unwrap_or_default(),
            },
            (None, Some(id), Some(ok)) => MsgKind::Result { id, ok },
            _ => MsgKind::Garbage,
        }
    }
}

/// 把 worker 回来的错误码翻成本 crate 的 [`Error`](crate::error::Error)。
///
/// 分级即预算:`Decode`/`BadRequest` **确定**怪这张图 → 帧级 `Error::Ocr`
/// (消耗重试预算);`Engine`(可能是设备重置/session 失效等引擎级故障,
/// 与单图触发无法区分)与 `ModelsMissing`/`Unknown` → 设施级 `Error::OcrInfra`,
/// 帧保持待处理、熔断器可感知。宁可让一张真会弄崩推理的图永远留在待处理里
/// (每批一次廉价失败,可见),也不能让一次引擎故障连坐烧掉一批好帧
/// (2026-07-17 实际发生:243 帧烧穿预算,截图随保留策略删除,不可恢复)。
pub fn err_from_wire(code: ErrCode, msg: &str) -> crate::error::Error {
    match code {
        ErrCode::RuntimeMissing => crate::error::Error::EmbeddingRuntimeMissing,
        ErrCode::Decode | ErrCode::BadRequest => {
            crate::error::Error::Ocr(format!("worker: {msg} ({code:?})"))
        }
        ErrCode::Engine | ErrCode::ModelsMissing | ErrCode::Unknown => {
            crate::error::Error::OcrInfra(format!("worker 引擎级错误: {msg} ({code:?})"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let req = WireReq::recognize(42, PathBuf::from("/a/b.jpg"));
        let s = serde_json::to_string(&req).unwrap();
        assert!(!s.contains('\n'), "NDJSON:一条消息必须是单行");
        let back: WireReq = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, 42);
        assert_eq!(back.op, ReqOp::Recognize);
        assert_eq!(back.path.unwrap(), PathBuf::from("/a/b.jpg"));
    }

    #[test]
    fn result_roundtrip_preserves_box_norm() {
        let lines = vec![OcrLine {
            text: "第一行".into(),
            box_norm: Some([0.1, 0.2, 0.3, 0.04]),
        }];
        let s = serde_json::to_string(&WireMsg::result_ok(7, lines)).unwrap();
        let back: WireMsg = serde_json::from_str(&s).unwrap();
        match back.classify() {
            MsgKind::Result { id: 7, ok: true } => {}
            other => panic!("形态错了: {other:?}"),
        }
        assert_eq!(back.lines.len(), 1);
        assert_eq!(back.lines[0].text, "第一行");
        // memory_locate 靠 box 定位——协议丢了它,搜索页 lightbox 就瞎了
        assert_eq!(back.lines[0].box_norm, Some([0.1, 0.2, 0.3, 0.04]));
    }

    #[test]
    fn classify_forms() {
        assert!(matches!(
            WireMsg::ready("vision").classify(),
            MsgKind::Ready { .. }
        ));
        assert!(matches!(
            WireMsg::fatal(ErrCode::ModelsMissing, "缺模型".into()).classify(),
            MsgKind::Fatal {
                code: ErrCode::ModelsMissing,
                ..
            }
        ));
        assert!(matches!(
            WireMsg::result_err(3, ErrCode::Decode, "读图失败".into()).classify(),
            MsgKind::Result { id: 3, ok: false }
        ));
        // 结构合法但语义不完整 → Garbage,父进程跳过而不是报错
        let half: WireMsg = serde_json::from_str(r#"{"v":1}"#).unwrap();
        assert!(matches!(half.classify(), MsgKind::Garbage));
    }

    #[test]
    fn unknown_err_code_is_forward_compatible() {
        let m: WireMsg =
            serde_json::from_str(r#"{"v":1,"id":1,"ok":false,"code":"weird_new_code"}"#).unwrap();
        assert_eq!(m.code, Some(ErrCode::Unknown));
    }

    #[test]
    fn wire_error_grading_protects_frame_budget() {
        // 确定怪图的 → 帧级(烧重试预算)
        assert!(matches!(
            err_from_wire(ErrCode::Decode, "坏图"),
            crate::error::Error::Ocr(_)
        ));
        assert!(matches!(
            err_from_wire(ErrCode::BadRequest, "缺 path"),
            crate::error::Error::Ocr(_)
        ));
        // 可能是引擎坏的 → 设施级(不烧预算、熔断器可感知)。
        // 这条边界错了会重演 07-17:引擎故障连坐烧穿一批好帧的预算
        assert!(matches!(
            err_from_wire(ErrCode::Engine, "session 失效"),
            crate::error::Error::OcrInfra(_)
        ));
        assert!(matches!(
            err_from_wire(ErrCode::Unknown, "新版 worker 的新码"),
            crate::error::Error::OcrInfra(_)
        ));
        assert!(matches!(
            err_from_wire(ErrCode::RuntimeMissing, "no ort"),
            crate::error::Error::EmbeddingRuntimeMissing
        ));
    }
}
