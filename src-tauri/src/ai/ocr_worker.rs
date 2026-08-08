//! OCR worker 子进程的**子进程侧**:`--ocr-worker` 分叉进来后的全部人生。
//!
//! 职责刻意压到最小:加载识别引擎 → 报 ready → 逐行收请求、逐行回结果,
//! 直到 stdin EOF(父进程死了)或收到 shutdown。**不碰** DB、托盘、Tauri、
//! 单实例插件、采集、同步——它只是一台识别机器。
//!
//! 三道孤儿网(macOS 的 job_guard 防不住强杀父进程,这里自己兜):
//! 1. stdin EOF → 正常退出(父进程无论怎么死,内核都会关掉管道);
//! 2. 看门狗线程:单个请求超过 [`SELF_DEADLINE`] 没完成 → `abort()`——
//!    专治"引擎挂死 + 父进程也没了"的双重死局,EOF 都读不到的那种;
//! 3. 空闲超过 [`IDLE_EXIT`] → 自行退出,兜底一切没想到的路径。
//!
//! 看门狗必须是 `std::thread` 而非 tokio 任务:引擎挂死时阻塞的是运行时
//! 本身(single-thread runtime 上 recognize 是同步调用),tokio 任务一并
//! 饿死,只有独立线程还醒着。

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use super::ocr::{OcrEngine, OcrLine};
use super::ocr_proto::{ErrCode, ReqOp, WireMsg, WireReq};
use crate::error::{Error, Result};

/// 单个请求的自毁时限。**必须大于父进程的请求超时(90s)**:正常情况下父进程
/// 先超时、先动手杀;这条只在父进程已经死了、没人来杀的时候兜底。
const SELF_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);

/// 空闲自退时限。父进程的空闲回收(60s/600s)先生效;这条同样是孤儿兜底。
const IDLE_EXIT: std::time::Duration = std::time::Duration::from_secs(900);

/// 识别引擎的最小面:让 [`serve`] 能被假引擎驱动(含"故意永久挂死"的那个),
/// 协议循环因此可以在无模型、无真实引擎的环境里被完整测试。
pub(crate) trait Recognize: Send + Sync {
    fn recognize_file(&self, p: &Path) -> Result<Vec<OcrLine>>;
}

impl Recognize for OcrEngine {
    fn recognize_file(&self, p: &Path) -> Result<Vec<OcrLine>> {
        OcrEngine::recognize_file(self, p)
    }
}

/// 看门狗共享状态。0 = 空闲;非 0 = 当前请求的自毁时限(单调毫秒)。
pub(crate) struct Watchdog {
    deadline_ms: Arc<AtomicU64>,
    idle_since_ms: Arc<AtomicU64>,
}

fn mono_ms() -> u64 {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    BASE.get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

impl Watchdog {
    pub(crate) fn new() -> Self {
        Self {
            deadline_ms: Arc::new(AtomicU64::new(0)),
            idle_since_ms: Arc::new(AtomicU64::new(mono_ms())),
        }
    }

    fn busy(&self) {
        self.deadline_ms.store(
            mono_ms() + SELF_DEADLINE.as_millis() as u64,
            Ordering::Relaxed,
        );
    }

    fn idle(&self) {
        self.deadline_ms.store(0, Ordering::Relaxed);
        self.idle_since_ms.store(mono_ms(), Ordering::Relaxed);
    }

    /// 起监督线程。每秒看一眼;发现越限直接了断进程——这里没有优雅可言,
    /// 优雅路径(EOF/shutdown)根本走不到这儿。
    fn spawn_thread(&self) {
        let deadline = Arc::clone(&self.deadline_ms);
        let idle_since = Arc::clone(&self.idle_since_ms);
        std::thread::Builder::new()
            .name("ocr-worker-watchdog".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let now = mono_ms();
                let d = deadline.load(Ordering::Relaxed);
                if d != 0 && now > d {
                    eprintln!(
                        "[ocr-worker] 请求超过 {}s 未完成且无人来杀,自毁",
                        SELF_DEADLINE.as_secs()
                    );
                    std::process::abort();
                }
                if d == 0
                    && now.saturating_sub(idle_since.load(Ordering::Relaxed))
                        > IDLE_EXIT.as_millis() as u64
                {
                    eprintln!("[ocr-worker] 空闲超过 {}s,自行退出", IDLE_EXIT.as_secs());
                    std::process::exit(0);
                }
            })
            .expect("看门狗线程必须起得来");
    }
}

/// 把引擎错误翻成线上错误码。分级决定父进程烧不烧该帧的重试预算:
/// - `Decode` = 图本身读不出来(损坏/半写入/0 尺寸)——**确定**是这张图的问题;
/// - `Engine` = 推理层错误——设备重置、session 失效等**引擎级**故障也长这样,
///   与"这张图触发的推理错误"无法区分,父进程按设施处理(不烧帧、重建 worker)。
///   曾把两者混为帧级:一次引擎故障连坐烧穿 243 帧的重试预算(2026-07-17),
///   截图随保留策略删除,文字不可恢复。
///
/// 判据是错误文本标记("读图失败" = Paddle 解码;"zero-dimension" = Vision
/// 对 0 尺寸图的报错;"检测框数量异常" = det 误检爆炸熔断)——脆弱但生成点
/// 都在本仓库内且有测试钉住,改动生成点必须同步这里。
fn wire_code(e: &Error) -> ErrCode {
    match e {
        Error::EmbeddingRuntimeMissing => ErrCode::RuntimeMissing,
        // "检测框数量异常"/"识别单元数量异常" = ocr.rs 的两道熔断(框数爆炸 /
        // 几何异常切出海量段):虽发生在推理后,但归责与解码失败同路——确定是
        // 这张图的问题,必须烧帧,否则毒帧无限重试堵死整个队列(issue #26)。
        Error::Ocr(s)
            if s.contains("读图失败")
                || s.contains("zero-dimension")
                || s.contains("检测框数量异常")
                || s.contains("识别单元数量异常") =>
        {
            ErrCode::Decode
        }
        _ => ErrCode::Engine,
    }
}

/// 协议主循环。与传输解耦:生产喂 stdin/stdout,测试喂 `tokio::io::duplex`。
/// 返回 `Ok(())` = stdin EOF 或收到 shutdown,都是正常退出路径。
pub(crate) async fn serve<R, W>(
    rx: R,
    mut tx: W,
    engine: &dyn Recognize,
    wd: &Watchdog,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(rx);
    let mut buf = String::new();
    loop {
        buf.clear();
        if lines.read_line(&mut buf).await? == 0 {
            return Ok(()); // EOF:父进程走了,跟着走
        }
        let req: WireReq = match serde_json::from_str(buf.trim_end()) {
            Ok(r) => r,
            Err(e) => {
                // 父进程不该发垃圾;万一发了,跳过比死循环/退出都好
                eprintln!("[ocr-worker] 无法解析的请求行,跳过: {e}");
                continue;
            }
        };
        match req.op {
            ReqOp::Shutdown => return Ok(()),
            ReqOp::Recognize => {
                let resp = match &req.path {
                    None => WireMsg::result_err(req.id, ErrCode::BadRequest, "缺 path".into()),
                    Some(p) => {
                        wd.busy();
                        let r = engine.recognize_file(p);
                        wd.idle();
                        match r {
                            Ok(lines) => WireMsg::result_ok(req.id, lines),
                            Err(e) => WireMsg::result_err(req.id, wire_code(&e), e.to_string()),
                        }
                    }
                };
                let mut line = serde_json::to_string(&resp).expect("协议类型必可序列化");
                line.push('\n');
                tx.write_all(line.as_bytes()).await?;
                tx.flush().await?;
            }
        }
    }
}

/// 测试钩子:`HINDSIGHT_OCR_WORKER_HANG=1` 时,第一次识别永久挂死。
/// 用来在真实进程层面验证父进程的"超时 → 杀 → 重建"链路——这正是本次
/// 事故里最需要证明的行为,没法指望 Vision 在测试时配合地挂一次。
struct HangFirst<E> {
    inner: E,
    hung: std::sync::atomic::AtomicBool,
}

impl<E: Recognize> Recognize for HangFirst<E> {
    fn recognize_file(&self, p: &Path) -> Result<Vec<OcrLine>> {
        if !self.hung.swap(true, Ordering::SeqCst) {
            eprintln!("[ocr-worker] HANG 钩子触发:本请求将永不返回");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        self.inner.recognize_file(p)
    }
}

/// 进程入口。永不返回。
///
/// 初始化顺序有讲究:日志(stderr!stdout 是协议信道)→ ORT dylib 路径
/// (Paddle 后端建 session 前必须设好)→ 看门狗 → 引擎 → ready 握手 → 主循环。
pub fn run_worker(fast: bool, parent_pid: Option<u32>) -> ! {
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("hindsight=info"),
    )
    .target(env_logger::Target::Stderr)
    .try_init();

    // 看门狗先于**一切**文件系统访问:数据目录可能在网络卷上,下面的 dylib
    // 路径解析可能阻塞在 syscall 里——那个窗口里若父进程已死,必须已有一道
    // 孤儿网在岗(空闲自退计时从此刻起算)。
    let wd = Watchdog::new();
    wd.spawn_thread();

    // 收养检测(unix):父进程可能在 spawn 与此刻之间死掉——那种情况下
    // stdin 的读端未必成形,EOF 网不可靠;被收养 = 父已死,直接退。
    // (Linux 的 PDEATHSIG 设置同样存在设置前父死的窗口,这一步把它补上。)
    #[cfg(unix)]
    if let Some(pp) = parent_pid {
        let actual = std::os::unix::process::parent_id();
        if actual != pp {
            eprintln!("[ocr-worker] 已被收养(父 {pp} → {actual}),父进程已死,退出");
            std::process::exit(0);
        }
    }
    #[cfg(not(unix))]
    let _ = parent_pid; // Windows 由 Job Object kill-on-close 兜底

    super::embedding_runtime::init_dylib_path();

    let engine = match if fast {
        OcrEngine::load_fast()
    } else {
        OcrEngine::load()
    } {
        Ok(e) => e,
        Err(e) => {
            let code = match &e {
                Error::EmbeddingRuntimeMissing => ErrCode::RuntimeMissing,
                Error::Io(_) => ErrCode::ModelsMissing,
                _ => ErrCode::Engine,
            };
            emit_stdout(&WireMsg::fatal(code, e.to_string()));
            std::process::exit(1);
        }
    };
    emit_stdout(&WireMsg::ready(engine.backend_name()));
    log::info!(
        "[ocr-worker] 就绪 backend={} fast={fast} pid={}",
        engine.backend_name(),
        std::process::id()
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("worker 运行时必须起得来");
    let result = if std::env::var_os("HINDSIGHT_OCR_WORKER_HANG").is_some() {
        let hang = HangFirst {
            inner: engine,
            hung: std::sync::atomic::AtomicBool::new(false),
        };
        rt.block_on(serve(tokio::io::stdin(), tokio::io::stdout(), &hang, &wd))
    } else {
        rt.block_on(serve(tokio::io::stdin(), tokio::io::stdout(), &engine, &wd))
    };
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("[ocr-worker] 管道异常退出: {e}");
            std::process::exit(1);
        }
    }
}

/// 同步写一行协议到 stdout 并 flush。stdout 接管道时是块缓冲,
/// 不 flush 的话 ready 行会一直躺在缓冲里,父进程握手白等到超时。
fn emit_stdout(msg: &WireMsg) {
    use std::io::Write;
    let mut line = serde_json::to_string(msg).expect("协议类型必可序列化");
    line.push('\n');
    let mut out = std::io::stdout();
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Canned {
        calls: AtomicUsize,
    }

    impl Recognize for Canned {
        fn recognize_file(&self, p: &Path) -> Result<Vec<OcrLine>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if name.contains("unreadable") {
                return Err(Error::Ocr("读图失败: 半写入的截图".into()));
            }
            if name.contains("bad") {
                return Err(Error::Ocr("这张图坏了".into()));
            }
            Ok(vec![OcrLine {
                text: format!("来自 {name}"),
                box_norm: Some([0.1, 0.2, 0.3, 0.4]),
            }])
        }
    }

    /// 驱动 serve:写入若干请求行,收集全部输出行。
    async fn drive(input: &str) -> (Vec<WireMsg>, std::io::Result<()>) {
        let engine = Canned {
            calls: AtomicUsize::new(0),
        };
        let wd = Watchdog::new(); // 不起线程:测试只要状态机
        let (rx, mut feed) = tokio::io::duplex(64 * 1024);
        let (mut sink, tx) = tokio::io::duplex(64 * 1024);
        use tokio::io::AsyncReadExt;
        feed.write_all(input.as_bytes()).await.unwrap();
        drop(feed); // EOF
        let served = serve(rx, tx, &engine, &wd).await;
        let mut out = String::new();
        sink.read_to_string(&mut out).await.unwrap();
        let msgs = out
            .lines()
            .map(|l| serde_json::from_str::<WireMsg>(l).expect("worker 输出必须行行合法"))
            .collect();
        (msgs, served)
    }

    #[tokio::test]
    async fn roundtrip_ok_and_frame_error_keep_loop_alive() {
        let good = serde_json::to_string(&WireReq::recognize(1, "/a/good.jpg".into())).unwrap();
        let bad = serde_json::to_string(&WireReq::recognize(2, "/a/bad.jpg".into())).unwrap();
        let again = serde_json::to_string(&WireReq::recognize(3, "/a/good2.jpg".into())).unwrap();
        let (msgs, served) = drive(&format!("{good}\n{bad}\n{again}\n")).await;

        served.unwrap();
        assert_eq!(msgs.len(), 3, "三问三答:坏图绝不终止循环");
        assert!(matches!(
            msgs[0].classify(),
            super::super::ocr_proto::MsgKind::Result { id: 1, ok: true }
        ));
        assert_eq!(msgs[0].lines[0].box_norm, Some([0.1, 0.2, 0.3, 0.4]));
        assert!(matches!(
            msgs[1].classify(),
            super::super::ocr_proto::MsgKind::Result { id: 2, ok: false }
        ));
        // 推理层错误 → Engine(父进程按设施处理,不烧帧预算)
        assert_eq!(msgs[1].code, Some(ErrCode::Engine));
        assert!(matches!(
            msgs[2].classify(),
            super::super::ocr_proto::MsgKind::Result { id: 3, ok: true }
        ));
    }

    /// 解码错误(确定怪图)与推理错误(可能是引擎坏)必须分级——
    /// 这是 07-17 数据丢失事故的第二道回归钉(第一道在 digest 的分类测试里)。
    #[tokio::test]
    async fn decode_error_graded_separately_from_engine_error() {
        let dec =
            serde_json::to_string(&WireReq::recognize(1, "/a/unreadable.jpg".into())).unwrap();
        let eng = serde_json::to_string(&WireReq::recognize(2, "/a/bad.jpg".into())).unwrap();
        let (msgs, served) = drive(&format!(
            "{dec}
{eng}
"
        ))
        .await;
        served.unwrap();
        assert_eq!(msgs[0].code, Some(ErrCode::Decode), "读图失败 → 帧级");
        assert_eq!(msgs[1].code, Some(ErrCode::Engine), "推理错误 → 设施级");
    }

    /// det 误检爆炸熔断(ocr.rs 的 DET_MAX_BOXES)必须按帧级归责:
    /// 归成设施级(Engine)就不烧帧预算,毒帧会永远排在队首无限重试,
    /// 整个识别队列被一张图堵死(issue #26 实际发生,停摆约 24 小时)。
    /// 钉的是 wire_code 的文本标记匹配——ocr.rs 改错误措辞会在这里断。
    #[test]
    fn box_flood_fuse_graded_as_frame_level() {
        let e = Error::Ocr("检测框数量异常: 48231 框(熔断线 1000),疑似大面积纹理误检".into());
        assert_eq!(
            wire_code(&e),
            ErrCode::Decode,
            "误检爆炸 → 帧级,烧预算后放行队列"
        );
    }

    /// 单元数熔断(ocr.rs 的 REC_MAX_UNITS)同样必须帧级归责——issue #26 的
    /// 毒帧只有 11 框,框数熔断不触发,拦住它的正是这道。归成设施级就会
    /// 无限重试堵死队列。钉 wire_code 的文本标记,ocr.rs 改措辞在这里断。
    #[test]
    fn unit_flood_fuse_graded_as_frame_level() {
        let e = Error::Ocr("识别单元数量异常: 859004 单元(熔断线 10000),疑似检测框几何异常".into());
        assert_eq!(
            wire_code(&e),
            ErrCode::Decode,
            "几何异常爆炸 → 帧级,烧预算后放行队列"
        );
    }

    #[tokio::test]
    async fn garbage_request_skipped_missing_path_answered() {
        let no_path = r#"{"v":1,"id":9,"op":"recognize"}"#;
        let ok = serde_json::to_string(&WireReq::recognize(10, "/a/good.jpg".into())).unwrap();
        let (msgs, served) = drive(&format!("这不是JSON\n{no_path}\n{ok}\n")).await;

        served.unwrap();
        assert_eq!(msgs.len(), 2, "垃圾行静默跳过,不回话也不退出");
        assert_eq!(msgs[0].code, Some(ErrCode::BadRequest));
        assert!(matches!(
            msgs[1].classify(),
            super::super::ocr_proto::MsgKind::Result { id: 10, ok: true }
        ));
    }

    #[tokio::test]
    async fn shutdown_and_eof_both_end_cleanly() {
        let bye = serde_json::to_string(&WireReq::shutdown(1)).unwrap();
        let after = serde_json::to_string(&WireReq::recognize(2, "/a/good.jpg".into())).unwrap();
        let (msgs, served) = drive(&format!("{bye}\n{after}\n")).await;
        served.unwrap();
        assert!(msgs.is_empty(), "shutdown 后的请求不再处理");

        let (msgs, served) = drive("").await;
        served.unwrap();
        assert!(msgs.is_empty(), "空输入 = 立即 EOF,干净退出");
    }
}
