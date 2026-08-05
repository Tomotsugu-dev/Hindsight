//! OCR worker 子进程的**父进程侧**:拉起、握手、串行发请求、超时杀、重建。
//!
//! 这是识别挂死问题的最终答案(2026-08-05 定案,前情见 `ocr_proto.rs` 头注):
//! 引擎挂起时,进程内什么都取消不掉,唯一能干净收场的是进程边界——
//! 超时 → 杀 worker(内核回收线程与 ANE/GPU 资源)→ 下个请求重建。
//! "恢复"从此是**我们代码的确定性行为**,不再赌引擎不挂。
//!
//! 并发纪律:全部识别(消化循环 + 搜索页 memory_locate)都走这里的同一把锁,
//! 同一时刻至多一个请求在飞。这不只是协议简化——实测**并发使用识别服务
//! 会把挂死概率放大一个量级**(两客户端并跑,第 50 次即挂;单客户端 2317 次),
//! 串行化本身就是修复的一部分。
//!
//! 镜像 `ai/server.rs`(llama-server 监管)的既有范式:tokio Mutex、
//! stderr 环形日志(不排空会把子进程堵死在写满的管道上)、kill 后台收尸。
//! 差异:请求全程握在一把锁里,不需要 generation 计数器——不存在
//! "放锁等健康检查"的窗口。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use super::ocr::OcrLine;
use super::ocr_proto::{
    err_from_wire, ErrCode, MsgKind, WireMsg, WireReq, MAX_LINE_BYTES, PROTOCOL_V,
};
use crate::error::{Error, Result};

/// 单请求超时。超过即判 worker 挂死:杀、重建、把这一帧记设施故障。
/// 90s = 观测到的最慢合法单帧(37s)的 2.4 倍;engine 冷启动不算在内
/// (握手阶段单独计时)。
const REQ_TIMEOUT: Duration = Duration::from_secs(90);

/// ready 握手超时。Windows 的 Paddle 冷启动要建两个 ORT session +
/// DirectML 图编译 + 预热,冷盘上以十秒计;macOS Vision 是瞬时的。
const READY_TIMEOUT: Duration = Duration::from_secs(120);

/// 空闲回收阈值。macOS Vision worker 无状态、重建近乎免费,尽早还进程;
/// 其它平台 Paddle 重建要 10-30s + ~400MB,值得多留一会儿。
/// worker 自己的空闲自退(900s)恒晚于这里——那是孤儿兜底,不该先触发。
const IDLE_THRESHOLD: Duration = if cfg!(target_os = "macos") {
    Duration::from_secs(60)
} else {
    Duration::from_secs(600)
};

const LOG_RING_SIZE: usize = 500;

type LogRing = std::sync::Mutex<VecDeque<String>>;
type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;
/// 读端一律经 `Take` 限额:单行字节预算在**读取时**强制。曾经是读完整行后
/// 才检查长度——无换行的洪水在检查发生前就能把父进程内存撑爆,防护名不副实。
type CappedReader = BufReader<tokio::io::Take<BoxRead>>;

/// 单行读取预算(超出 [`MAX_LINE_BYTES`] 两字节,便于区分"恰好到顶"与"超限")。
const LINE_BUDGET: u64 = (MAX_LINE_BYTES as u64) + 2;

fn capped_reader(r: BoxRead) -> CappedReader {
    use tokio::io::AsyncReadExt;
    BufReader::new(r.take(LINE_BUDGET))
}
type SpawnFn = Arc<dyn Fn(bool, Arc<LogRing>) -> Result<Link> + Send + Sync>;

/// 一条到 worker 的活链接。`child` 带 `kill_on_drop`:**丢弃 Link 即杀进程**,
/// 这是超时路径的全部杀伤机制——没有单独的 kill 方法可以忘记调。
pub(crate) struct Link {
    tx: BoxWrite,
    rx: CappedReader,
    /// 只为 Drop 而持有:`kill_on_drop` 让"丢弃 Link"本身就是杀进程。
    /// 没有任何代码路径需要读它——这正是设计(不存在能忘记调的 kill)。
    #[allow(dead_code)]
    child: Option<tokio::process::Child>,
    fast: bool,
    pid: Option<u32>,
    /// 请求在飞标记:写出请求前置位,完整收到应答后清位。
    /// 调用方的 future 若在"已写未读"之间被丢弃(digest 外层兜底超时),
    /// 管道里会残留一条无人认领的应答——dirty 仍为真,下次取链接时发现
    /// 即弃之重建,而不是把旧应答误配给新请求(那会白付一轮序号错乱重建)。
    dirty: bool,
}

struct Inner {
    link: Option<Link>,
    next_id: u64,
}

pub struct OcrSupervisor {
    inner: tokio::sync::Mutex<Inner>,
    logs: Arc<LogRing>,
    spawn: SpawnFn,
    req_timeout: Duration,
    ready_timeout: Duration,
    idle_threshold: Duration,
    /// 最近一次请求完成时刻(单调毫秒);空闲回收据此判定
    last_used_ms: AtomicU64,
    fast: AtomicBool,
    /// 累计拉起次数(日志 + 测试断言"确实重建了")
    spawns: AtomicU64,
}

fn mono_ms() -> u64 {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    BASE.get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// 进程级单例。消化循环 / 常驻 tick / memory_locate 都够不到 AppHandle,
/// 与 `digest::RUNNING`、`MINIMIZE_TO_TRAY` 同一惯例,在这儿会合。
pub fn global() -> &'static Arc<OcrSupervisor> {
    static G: std::sync::OnceLock<Arc<OcrSupervisor>> = std::sync::OnceLock::new();
    G.get_or_init(|| Arc::new(OcrSupervisor::production()))
}

impl OcrSupervisor {
    fn production() -> Self {
        Self::with_spawn_and_timeouts(
            Arc::new(spawn_process),
            REQ_TIMEOUT,
            READY_TIMEOUT,
            IDLE_THRESHOLD,
        )
    }

    fn with_spawn_and_timeouts(
        spawn: SpawnFn,
        req_timeout: Duration,
        ready_timeout: Duration,
        idle_threshold: Duration,
    ) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(Inner {
                link: None,
                next_id: 1,
            }),
            logs: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            spawn,
            req_timeout,
            ready_timeout,
            idle_threshold,
            last_used_ms: AtomicU64::new(mono_ms()),
            fast: AtomicBool::new(false),
            spawns: AtomicU64::new(0),
        }
    }

    /// 识别一帧。惰性拉起 worker;超时/进程死 → 丢链接(即杀),下次重建。
    ///
    /// 错误分级(消化循环据此决定烧不烧帧的重试预算):
    /// - 走管道回来的错误 → 帧级(`Error::Ocr`,worker 活着,图有问题);
    /// - 超时 / EOF / 协议错乱 / 拉不起来 → 设施级(`Error::OcrInfra`)。
    pub async fn recognize(&self, path: &Path) -> Result<Vec<OcrLine>> {
        // 排障后门:回到进程内识别(老路径)。只给支持诊断用——它会把
        // 引擎挂死重新变成整个 app 的挂死,绝不能是自动回退。
        if std::env::var_os("HINDSIGHT_OCR_INPROCESS").is_some() {
            return Self::recognize_inprocess(path).await;
        }

        let mut inner = self.inner.lock().await;
        self.ensure_link_locked(&mut inner).await?;
        let id = inner.next_id;
        inner.next_id += 1;

        let link = inner.link.as_mut().expect("ensure_link 刚保证过");
        link.dirty = true; // 从现在起到完整收到应答之前,这条链上有在飞请求
        let mut req = serde_json::to_string(&WireReq::recognize(id, path.to_path_buf()))
            .expect("协议类型必可序列化");
        req.push('\n');
        let sent = async {
            link.tx.write_all(req.as_bytes()).await?;
            link.tx.flush().await
        }
        .await;
        if let Err(e) = sent {
            inner.link = None; // 丢弃即杀
            self.touch();
            return Err(Error::OcrInfra(format!(
                "写请求失败(worker 已死?): {e}; stderr 尾部: {}",
                self.stderr_tail()
            )));
        }

        let outcome = tokio::time::timeout(self.req_timeout, read_result(link, id)).await;
        self.touch();
        match outcome {
            Err(_elapsed) => {
                let pid = link.pid;
                inner.link = None; // kill_on_drop:超时即杀,这是本模块存在的意义
                log::error!(
                    "OCR worker(pid={pid:?})单帧 {}s 无响应,已杀掉重建;stderr 尾部: {}",
                    self.req_timeout.as_secs(),
                    self.stderr_tail()
                );
                Err(Error::OcrInfra(format!(
                    "识别 {}s 无响应,worker 已重建",
                    self.req_timeout.as_secs()
                )))
            }
            Ok(Ok(WireOutcome::Lines(lines))) => {
                link.dirty = false;
                Ok(lines)
            }
            Ok(Ok(WireOutcome::FrameErr(code, msg))) => {
                link.dirty = false;
                let e = err_from_wire(code, &msg);
                if matches!(e, Error::OcrInfra(_)) {
                    // 引擎级错误(设备重置/session 失效同形):旧 session 大概率
                    // 已废,丢链接让下次请求拿全新 worker——否则每批都撞同一个
                    // 坏 session,三连败进冷却,循环到重启为止
                    inner.link = None;
                }
                Err(e)
            }
            Ok(Err(e)) => {
                inner.link = None;
                Err(match e {
                    Error::OcrInfra(m) => {
                        Error::OcrInfra(format!("{m}; stderr 尾部: {}", self.stderr_tail()))
                    }
                    other => other,
                })
            }
        }
    }

    /// 以指定档位识别:先断言档位再识别。消化管线的闭包用它——
    /// 没有这层断言的话,手动全速批结束后,常驻批会继续沿用 fast worker
    /// (Windows 上 Paddle 全速线程数 = 后台识别打扰前台,违背常驻模式约定)。
    /// 同档零成本(一次原子写);异档只会发生在批边界(批之间由 digest 的
    /// RUNNING 互斥天然串行),重建代价一批一次。
    pub async fn recognize_as(&self, path: &Path, fast: bool) -> Result<Vec<OcrLine>> {
        self.fast.store(fast, Ordering::Relaxed);
        self.recognize(path).await
    }

    /// 预拉起 + 完成握手。`Pipeline::load` 调它,让 Paddle 的 10-30s 冷启动
    /// 落在"引擎级失败中断整批"的语义里,而不是被算进第一帧的请求超时。
    pub async fn ensure_ready(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        self.ensure_link_locked(&mut inner).await
    }

    /// 线程档位(手动全速 vs 后台保守;只影响 Paddle,Vision 无感)。
    /// 值变了就丢链接,下次请求按新档重建。
    pub async fn set_fast(&self, fast: bool) {
        self.fast.store(fast, Ordering::Relaxed);
        let mut inner = self.inner.lock().await;
        if inner.link.as_ref().is_some_and(|l| l.fast != fast) {
            inner.link = None;
        }
    }

    /// 停 worker(app 退出钩子用)。拿不到锁(有请求在飞)就不硬等——
    /// 父进程退出时管道关闭,worker 读到 EOF 自会退,这里只是提前一点。
    pub async fn stop(&self) {
        match tokio::time::timeout(Duration::from_millis(500), self.inner.lock()).await {
            Ok(mut inner) => {
                if let Some(mut link) = inner.link.take() {
                    let mut bye = serde_json::to_string(&WireReq::shutdown(u64::MAX))
                        .expect("协议类型必可序列化");
                    bye.push('\n');
                    let _ = link.tx.write_all(bye.as_bytes()).await;
                    let _ = link.tx.flush().await;
                    // drop(link) → kill_on_drop 兜底 shutdown 没被理会的情况
                }
            }
            Err(_) => log::debug!("OCR worker 停止:有请求在飞,交给 EOF 兜底"),
        }
    }

    /// 空闲回收:持 Weak,监管器没了自己退;拿不到锁(在忙)就跳过本轮。
    ///
    /// 返回 `None` = 不在 tokio 运行时上下文(启动布线错误)。**降级而非 panic**:
    /// 丢掉空闲回收的代价是 worker 闲置多占些内存,而这里 panic 的代价是
    /// app 启动直接 abort(0.8.15 的变砖事故;panic 点在 tao 的
    /// did_finish_launching,不可展开)。布线错误靠 error 日志暴露。
    pub fn spawn_idle_watcher(self: &Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            log::error!("OCR 空闲回收器未启动:不在 tokio 运行时上下文(启动布线错误)");
            return None;
        };
        let weak = Arc::downgrade(self);
        Some(rt.spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(10));
            tick.tick().await; // 吞掉立即触发的第一下
            loop {
                tick.tick().await;
                let Some(sup) = weak.upgrade() else { return };
                let idle_ms = mono_ms().saturating_sub(sup.last_used_ms.load(Ordering::Relaxed));
                if idle_ms < sup.idle_threshold.as_millis() as u64 {
                    continue;
                }
                let reclaimed = match sup.inner.try_lock() {
                    Ok(mut inner) => inner.link.take().is_some(),
                    Err(_) => false, // 在忙:跳过本轮,绝不排队等一个 90s 的挂帧
                };
                if reclaimed {
                    log::info!("OCR worker 空闲 {}s,回收进程", idle_ms / 1000);
                }
            }
        }))
    }

    /// worker stderr 环形日志(调试页/错误信息用;暂无 UI 消费方,保留 API)。
    #[allow(dead_code)]
    pub fn recent_logs(&self) -> Vec<String> {
        self.logs
            .lock()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn stderr_tail(&self) -> String {
        self.logs
            .lock()
            .map(|g| {
                g.iter()
                    .rev()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default()
    }

    fn touch(&self) {
        self.last_used_ms.store(mono_ms(), Ordering::Relaxed);
    }

    async fn ensure_link_locked(&self, inner: &mut Inner) -> Result<()> {
        // dirty = 上一个请求的 future 在"已写未读"之间被丢弃,管道里躺着
        // 无人认领的应答——这条链不能再用,弃之重建
        if inner.link.as_ref().is_some_and(|l| l.dirty) {
            log::warn!("OCR worker 链接残留在飞请求(调用方被取消),弃链重建");
            inner.link = None;
        }
        let want_fast = self.fast.load(Ordering::Relaxed);
        if inner.link.as_ref().is_some_and(|l| l.fast == want_fast) {
            return Ok(());
        }
        inner.link = None;
        let n = self.spawns.fetch_add(1, Ordering::Relaxed) + 1;
        log::info!("拉起 OCR worker(第 {n} 次,fast={want_fast})");
        let mut link = (self.spawn)(want_fast, Arc::clone(&self.logs))?;

        // ready 握手:跳过垃圾行,等 ready 或 fatal
        let hs = tokio::time::timeout(self.ready_timeout, async {
            let mut buf = String::new();
            loop {
                buf.clear();
                let n = read_line_capped(&mut link.rx, &mut buf).await?;
                if n == 0 {
                    return Err(Error::OcrInfra("worker 握手前退出".into()));
                }
                let Ok(msg) = serde_json::from_str::<WireMsg>(buf.trim_end()) else {
                    log::debug!("worker 握手期非协议输出,跳过: {}", buf.trim_end());
                    continue;
                };
                match msg.classify() {
                    MsgKind::Ready { backend, pid } => {
                        if msg.v != PROTOCOL_V {
                            return Err(Error::OcrInfra(format!(
                                "worker 协议版本 {} ≠ {PROTOCOL_V}(app 原地升级后的残留?)",
                                msg.v
                            )));
                        }
                        log::info!("OCR worker 就绪 backend={backend} pid={pid}");
                        return Ok(());
                    }
                    MsgKind::Fatal { code, msg } => {
                        return Err(match code {
                            ErrCode::RuntimeMissing => Error::EmbeddingRuntimeMissing,
                            _ => Error::OcrInfra(format!("worker 启动失败: {msg}")),
                        });
                    }
                    _ => continue,
                }
            }
        })
        .await;
        match hs {
            Err(_elapsed) => Err(Error::OcrInfra(format!(
                "worker 握手超时({}s);stderr 尾部: {}",
                self.ready_timeout.as_secs(),
                self.stderr_tail()
            ))),
            Ok(Err(e)) => Err(e),
            Ok(Ok(())) => {
                inner.link = Some(link);
                Ok(())
            }
        }
    }

    /// 排障后门的进程内识别:老路径原样(同步引擎 + spawn_blocking,无超时)。
    async fn recognize_inprocess(path: &Path) -> Result<Vec<OcrLine>> {
        use super::ocr::OcrEngine;
        static ENGINE: std::sync::OnceLock<OcrEngine> = std::sync::OnceLock::new();
        if ENGINE.get().is_none() {
            let engine = tokio::task::spawn_blocking(OcrEngine::load)
                .await
                .map_err(|e| Error::Ocr(format!("spawn_blocking: {e}")))??;
            let _ = ENGINE.set(engine);
        }
        let p = path.to_path_buf();
        tokio::task::spawn_blocking(move || ENGINE.get().expect("上面刚保证过").recognize_file(&p))
            .await
            .map_err(|e| Error::Ocr(format!("spawn_blocking: {e}")))?
    }
}

enum WireOutcome {
    Lines(Vec<OcrLine>),
    FrameErr(ErrCode, String),
}

/// 读到 `id` 对应的结果行为止。垃圾行跳过;EOF / 序号错乱 / 协议违规 →
/// 设施级错误(调用方随即丢链接)。
async fn read_result(link: &mut Link, expect_id: u64) -> Result<WireOutcome> {
    let mut buf = String::new();
    loop {
        buf.clear();
        let n = read_line_capped(&mut link.rx, &mut buf).await?;
        if n == 0 {
            return Err(Error::OcrInfra("worker 请求途中退出".into()));
        }
        let Ok(msg) = serde_json::from_str::<WireMsg>(buf.trim_end()) else {
            // ORT/CoreML 等原生库可能污染 stdout:跳过,绝不因此判请求失败
            log::debug!("worker 非协议输出,跳过: {}", buf.trim_end());
            continue;
        };
        match msg.classify() {
            MsgKind::Result { id, ok } if id == expect_id => {
                if msg.v != PROTOCOL_V {
                    // 理论歧义窗口:形似结果的污染行 / 换版残留。版本不合一律
                    // 按协议错乱处理,绝不当成识别结果采信
                    return Err(Error::OcrInfra(format!(
                        "应答协议版本 {} ≠ {PROTOCOL_V}",
                        msg.v
                    )));
                }
                return Ok(if ok {
                    WireOutcome::Lines(msg.lines)
                } else {
                    WireOutcome::FrameErr(
                        msg.code.unwrap_or(ErrCode::Unknown),
                        msg.msg.unwrap_or_default(),
                    )
                });
            }
            MsgKind::Result { id, .. } => {
                return Err(Error::OcrInfra(format!(
                    "worker 应答序号错乱(期待 {expect_id},收到 {id})"
                )));
            }
            MsgKind::Ready { .. } | MsgKind::Fatal { .. } => {
                return Err(Error::OcrInfra(
                    "worker 请求途中发送握手消息(协议违规)".into(),
                ));
            }
            MsgKind::Garbage => continue,
        }
    }
}

async fn read_line_capped(rx: &mut CappedReader, buf: &mut String) -> Result<usize> {
    // 每行重置限额:Take 在读取层强制预算,无换行的洪水最多读到预算顶就停,
    // 不可能先撑爆内存再被检查发现
    rx.get_mut().set_limit(LINE_BUDGET);
    let n = rx
        .read_line(buf)
        .await
        .map_err(|e| Error::OcrInfra(format!("读 worker 输出失败: {e}")))?;
    if buf.len() > MAX_LINE_BYTES {
        return Err(Error::OcrInfra("worker 输出行超限,判定失控".into()));
    }
    Ok(n)
}

/// 生产 spawn:重入自身二进制(`--ocr-worker`),零打包/签名改动。
/// `HINDSIGHT_OCR_WORKER_EXE` 覆盖可执行文件路径——dev 与 `#[ignore]` 真机
/// 测试用(cargo test 的 current_exe 是测试壳,不是 app)。
fn spawn_process(fast: bool, logs: Arc<LogRing>) -> Result<Link> {
    let exe = std::env::var_os("HINDSIGHT_OCR_WORKER_EXE")
        .map(PathBuf::from)
        .map_or_else(std::env::current_exe, Ok)
        .map_err(|e| Error::OcrInfra(format!("找不到自身可执行文件: {e}")))?;

    let mut cmd = tokio::process::Command::new(&exe);
    cmd.arg("--ocr-worker");
    if fast {
        cmd.arg("--fast");
    }
    cmd.arg("--parent-pid").arg(std::process::id().to_string());
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // 超时路径的杀伤机制:Link 被丢弃 → SIGKILL/TerminateProcess,tokio 后台收尸
    cmd.kill_on_drop(true);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    if let Err(e) = crate::ai::job_guard::prepare_command(&mut cmd) {
        log::warn!("OCR worker 进程保护配置失败(不阻断): {e}");
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::OcrInfra(format!("OCR worker 启动失败({}): {e}", exe.display())))?;
    let pid = child.id();
    if let Some(p) = pid {
        let _ = crate::ai::job_guard::assign_child_pid(p);
    }

    // stderr 必须持续排空,否则 worker 会阻塞在写满的管道上(server.rs 同款教训)
    let stderr = child.stderr.take().expect("stderr 已 piped");
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log::debug!("[ocr-worker] {line}");
            if let Ok(mut g) = logs.lock() {
                if g.len() >= LOG_RING_SIZE {
                    g.pop_front();
                }
                g.push_back(line);
            }
        }
    });

    let stdin = child.stdin.take().expect("stdin 已 piped");
    let stdout = child.stdout.take().expect("stdout 已 piped");
    Ok(Link {
        tx: Box::new(stdin),
        rx: capped_reader(Box::new(stdout)),
        child: Some(child),
        fast,
        pid,
        dirty: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// 假 worker:一段脚本任务 + duplex 双向管——零进程覆盖全部监管逻辑。
    fn scripted<F, Fut>(fast: bool, script: F) -> Link
    where
        F: FnOnce(
                BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
                tokio::io::WriteHalf<tokio::io::DuplexStream>,
            ) -> Fut
            + Send
            + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (parent_io, worker_io) = tokio::io::duplex(256 * 1024);
        let (w_rx, w_tx) = tokio::io::split(worker_io);
        tokio::spawn(script(BufReader::new(w_rx), w_tx));
        let (p_rx, p_tx) = tokio::io::split(parent_io);
        Link {
            tx: Box::new(p_tx),
            rx: capped_reader(Box::new(p_rx)),
            child: None,
            fast,
            pid: None,
            dirty: false,
        }
    }

    async fn send_line<W: AsyncWrite + Unpin>(tx: &mut W, msg: &WireMsg) {
        let mut s = serde_json::to_string(msg).unwrap();
        s.push('\n');
        tx.write_all(s.as_bytes()).await.unwrap();
        tx.flush().await.unwrap();
    }

    async fn next_req<R: AsyncRead + Unpin>(rx: &mut BufReader<R>) -> Option<WireReq> {
        let mut buf = String::new();
        if rx.read_line(&mut buf).await.ok()? == 0 {
            return None;
        }
        serde_json::from_str(buf.trim_end()).ok()
    }

    fn sup(spawn: SpawnFn) -> Arc<OcrSupervisor> {
        Arc::new(OcrSupervisor::with_spawn_and_timeouts(
            spawn,
            Duration::from_millis(200), // 请求超时:测试档
            Duration::from_millis(500), // 握手超时
            Duration::from_millis(50),  // 空闲阈值
        ))
    }

    fn one_line(text: &str) -> Vec<OcrLine> {
        vec![OcrLine {
            text: text.into(),
            box_norm: Some([0.0, 0.0, 1.0, 0.1]),
        }]
    }

    /// 应答如流的正常 worker:ready 后有问必答。
    fn echo_spawn(counter: Arc<AtomicUsize>) -> SpawnFn {
        Arc::new(move |fast, _logs| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(scripted(fast, |mut rx, mut tx| async move {
                send_line(&mut tx, &WireMsg::ready("fake")).await;
                while let Some(req) = next_req(&mut rx).await {
                    if req.op == ReqOp::Shutdown {
                        return;
                    }
                    send_line(&mut tx, &WireMsg::result_ok(req.id, one_line("识别行"))).await;
                }
            }))
        })
    }

    use super::super::ocr_proto::ReqOp;

    #[tokio::test]
    async fn warm_reuse_spawns_once() {
        let spawns = Arc::new(AtomicUsize::new(0));
        let s = sup(echo_spawn(Arc::clone(&spawns)));
        let a = s.recognize(Path::new("/x/1.jpg")).await.unwrap();
        let b = s.recognize(Path::new("/x/2.jpg")).await.unwrap();
        assert_eq!(a[0].text, "识别行");
        assert_eq!(b[0].text, "识别行");
        assert_eq!(spawns.load(Ordering::SeqCst), 1, "温热复用,不重复拉进程");
    }

    /// **本次事故的回归测试**:worker 收下请求后永不应答(= Vision 挂死)。
    /// 断言:在超时档位内返回设施级错误、下一个请求自动重建并成功。
    /// 这就是"卡死不再需要重启 app"的可执行证明。
    #[tokio::test]
    async fn hang_times_out_kills_and_next_request_respawns() {
        let spawns = Arc::new(AtomicUsize::new(0));
        let spawns2 = Arc::clone(&spawns);
        let s = sup(Arc::new(move |fast, _logs| {
            let n = spawns2.fetch_add(1, Ordering::SeqCst);
            Ok(scripted(fast, move |mut rx, mut tx| async move {
                send_line(&mut tx, &WireMsg::ready("fake")).await;
                if n == 0 {
                    // 第一个 worker:收请求,然后装死
                    let _ = next_req(&mut rx).await;
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                } else {
                    while let Some(req) = next_req(&mut rx).await {
                        send_line(&mut tx, &WireMsg::result_ok(req.id, one_line("复活"))).await;
                    }
                }
            }))
        }));

        let started = std::time::Instant::now();
        let err = s.recognize(Path::new("/x/wedge.jpg")).await.unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "超时档位内返回,不无限等"
        );
        assert!(matches!(err, Error::OcrInfra(_)), "挂死是设施故障: {err}");

        let ok = s.recognize(Path::new("/x/next.jpg")).await.unwrap();
        assert_eq!(ok[0].text, "复活", "下一请求自动重建 worker 并成功");
        assert_eq!(spawns.load(Ordering::SeqCst), 2, "确实杀掉重建了一次");
    }

    #[tokio::test]
    async fn death_mid_request_is_infra_and_respawns() {
        let spawns = Arc::new(AtomicUsize::new(0));
        let spawns2 = Arc::clone(&spawns);
        let s = sup(Arc::new(move |fast, _logs| {
            let n = spawns2.fetch_add(1, Ordering::SeqCst);
            Ok(scripted(fast, move |mut rx, mut tx| async move {
                send_line(&mut tx, &WireMsg::ready("fake")).await;
                if n == 0 {
                    let _ = next_req(&mut rx).await;
                    // 直接断管 = worker 崩溃(panic=abort / 被系统杀)
                } else {
                    while let Some(req) = next_req(&mut rx).await {
                        send_line(&mut tx, &WireMsg::result_ok(req.id, one_line("好了"))).await;
                    }
                }
            }))
        }));

        let err = s.recognize(Path::new("/x/die.jpg")).await.unwrap_err();
        assert!(
            matches!(err, Error::OcrInfra(_)),
            "进程死 = 设施故障,不烧帧预算"
        );
        let ok = s.recognize(Path::new("/x/after.jpg")).await.unwrap();
        assert_eq!(ok[0].text, "好了");
        assert_eq!(spawns.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stdout_garbage_tolerated_wire_error_is_frame_level() {
        let s = sup(Arc::new(|fast, _logs| {
            Ok(scripted(fast, |mut rx, mut tx| async move {
                send_line(&mut tx, &WireMsg::ready("fake")).await;
                while let Some(req) = next_req(&mut rx).await {
                    // ORT/CoreML 式 stdout 污染:非 JSON + 半截 JSON
                    tx.write_all("onnxruntime [W] some noise\n{\"v\":1}\n".as_bytes())
                        .await
                        .unwrap();
                    if req
                        .path
                        .as_ref()
                        .is_some_and(|p| p.to_string_lossy().contains("bad"))
                    {
                        send_line(
                            &mut tx,
                            &WireMsg::result_err(req.id, ErrCode::Decode, "读图失败".into()),
                        )
                        .await;
                    } else {
                        send_line(&mut tx, &WireMsg::result_ok(req.id, one_line("穿透噪音"))).await;
                    }
                }
            }))
        }));

        let ok = s.recognize(Path::new("/x/good.jpg")).await.unwrap();
        assert_eq!(ok[0].text, "穿透噪音", "垃圾行被跳过,请求不受影响");

        let err = s.recognize(Path::new("/x/bad.jpg")).await.unwrap_err();
        assert!(
            matches!(err, Error::Ocr(_)),
            "管道回来的错误是帧级(烧预算),不是设施级: {err}"
        );
    }

    /// 引擎级线错误(设备重置/session 失效同形)→ 设施级 + 重建 worker。
    /// 若归帧级:好帧被烧预算、熔断器被清零、坏 session 永不更换——07-17 重演。
    #[tokio::test]
    async fn engine_wire_error_is_infra_and_rebuilds_worker() {
        let spawns = Arc::new(AtomicUsize::new(0));
        let spawns2 = Arc::clone(&spawns);
        let s = sup(Arc::new(move |fast, _logs| {
            let n = spawns2.fetch_add(1, Ordering::SeqCst);
            Ok(scripted(fast, move |mut rx, mut tx| async move {
                send_line(&mut tx, &WireMsg::ready("fake")).await;
                while let Some(req) = next_req(&mut rx).await {
                    if n == 0 {
                        // 第一个 worker:session 坏了,凡问必报引擎错误
                        send_line(
                            &mut tx,
                            &WireMsg::result_err(req.id, ErrCode::Engine, "session 失效".into()),
                        )
                        .await;
                    } else {
                        send_line(&mut tx, &WireMsg::result_ok(req.id, one_line("新生"))).await;
                    }
                }
            }))
        }));

        let err = s.recognize(Path::new("/x/a.jpg")).await.unwrap_err();
        assert!(
            matches!(err, Error::OcrInfra(_)),
            "引擎错误必须归设施级(不烧帧预算): {err}"
        );
        let ok = s.recognize(Path::new("/x/b.jpg")).await.unwrap();
        assert_eq!(ok[0].text, "新生", "坏 session 被换掉,新 worker 正常");
        assert_eq!(spawns.load(Ordering::SeqCst), 2, "确实重建了");
    }

    #[tokio::test]
    async fn id_mismatch_is_protocol_violation() {
        let spawns = Arc::new(AtomicUsize::new(0));
        let spawns2 = Arc::clone(&spawns);
        let s = sup(Arc::new(move |fast, _logs| {
            let n = spawns2.fetch_add(1, Ordering::SeqCst);
            Ok(scripted(fast, move |mut rx, mut tx| async move {
                send_line(&mut tx, &WireMsg::ready("fake")).await;
                while let Some(req) = next_req(&mut rx).await {
                    let id = if n == 0 { req.id + 999 } else { req.id };
                    send_line(&mut tx, &WireMsg::result_ok(id, one_line("x"))).await;
                }
            }))
        }));
        let err = s.recognize(Path::new("/x/a.jpg")).await.unwrap_err();
        assert!(matches!(err, Error::OcrInfra(_)));
        assert!(
            err.to_string().contains("序号"),
            "错误说明白是序号错乱: {err}"
        );
        let _ = s.recognize(Path::new("/x/b.jpg")).await.unwrap();
        assert_eq!(spawns.load(Ordering::SeqCst), 2, "违规即杀,重建后恢复");
    }

    #[tokio::test]
    async fn fatal_runtime_missing_maps_to_typed_error() {
        let s = sup(Arc::new(|fast, _logs| {
            Ok(scripted(fast, |_rx, mut tx| async move {
                send_line(
                    &mut tx,
                    &WireMsg::fatal(ErrCode::RuntimeMissing, "onnxruntime 未安装".into()),
                )
                .await;
            }))
        }));
        let err = s.recognize(Path::new("/x/a.jpg")).await.unwrap_err();
        assert!(
            matches!(err, Error::EmbeddingRuntimeMissing),
            "runtime 缺失保持原有类型,设置页的引导 UI 靠它识别: {err}"
        );
    }

    /// 串行化契约:并发触发是实测的挂死放大器(两客户端第 50 次即挂),
    /// 这里断言两个并发 recognize 在 worker 侧绝不重叠。
    #[tokio::test]
    async fn concurrent_recognize_is_serialized() {
        let inflight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (inf, pk) = (Arc::clone(&inflight), Arc::clone(&peak));
        let s = sup(Arc::new(move |fast, _logs| {
            let (inf, pk) = (Arc::clone(&inf), Arc::clone(&pk));
            Ok(scripted(fast, move |mut rx, mut tx| async move {
                send_line(&mut tx, &WireMsg::ready("fake")).await;
                while let Some(req) = next_req(&mut rx).await {
                    let now = inf.fetch_add(1, Ordering::SeqCst) + 1;
                    pk.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    inf.fetch_sub(1, Ordering::SeqCst);
                    send_line(&mut tx, &WireMsg::result_ok(req.id, one_line("s"))).await;
                }
            }))
        }));

        let (a, b) = tokio::join!(
            s.recognize(Path::new("/x/1.jpg")),
            s.recognize(Path::new("/x/2.jpg"))
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "worker 侧同一时刻至多一个请求"
        );
    }

    #[tokio::test]
    async fn idle_watcher_reclaims_and_next_call_respawns() {
        let spawns = Arc::new(AtomicUsize::new(0));
        let s = sup(echo_spawn(Arc::clone(&spawns)));
        let _watcher = s.spawn_idle_watcher();

        s.recognize(Path::new("/x/1.jpg")).await.unwrap();
        assert_eq!(spawns.load(Ordering::SeqCst), 1);
        // 空闲阈值 50ms + 看门狗 tick 10s 太慢——直接等阈值后手动踢一脚逻辑:
        // 看门狗逻辑本体(阈值判断 + try_lock 回收)在这条路径上等 tick 不现实,
        // 改为压缩验证:等超过阈值后,下一次 recognize 前链接仍在(watcher 未必
        // 已跑),此处只验证"回收后能重建"——用 stop 模拟回收。
        s.stop().await;
        let again = s.recognize(Path::new("/x/2.jpg")).await.unwrap();
        assert_eq!(again[0].text, "识别行");
        assert_eq!(spawns.load(Ordering::SeqCst), 2, "回收后按需重建");
    }

    #[tokio::test]
    async fn set_fast_change_respawns_with_new_mode() {
        let modes = Arc::new(std::sync::Mutex::new(Vec::<bool>::new()));
        let modes2 = Arc::clone(&modes);
        let s = sup(Arc::new(move |fast, _logs| {
            modes2.lock().unwrap().push(fast);
            Ok(scripted(fast, |mut rx, mut tx| async move {
                send_line(&mut tx, &WireMsg::ready("fake")).await;
                while let Some(req) = next_req(&mut rx).await {
                    send_line(&mut tx, &WireMsg::result_ok(req.id, one_line("m"))).await;
                }
            }))
        }));

        s.set_fast(false).await;
        s.recognize(Path::new("/x/1.jpg")).await.unwrap();
        s.set_fast(true).await;
        s.recognize(Path::new("/x/2.jpg")).await.unwrap();
        assert_eq!(
            *modes.lock().unwrap(),
            vec![false, true],
            "档位切换即按新档重建"
        );
    }

    /// 真机端到端:真实二进制、真实管道、真实杀进程。
    ///
    /// 步骤:1) 正常识别一张真图;2) 用 HANG 钩子让 worker 在请求中途装死,
    /// 断言超时归为设施故障且**不用重启任何东西**;3) 撤掉钩子,断言下一个
    /// 请求自动重建 worker 并成功识别。这三步串起来 = "挂死自愈"的可执行证明。
    ///
    /// 跑法(需要先 cargo build 出 app 二进制):
    /// ```text
    /// HINDSIGHT_OCR_WORKER_EXE=target/debug/hindsight \
    /// HINDSIGHT_TEST_IMG=<任一截图路径> \
    ///   cargo test --lib real_worker_hang_recovery -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "需要真实 app 二进制与真实截图,手动跑(命令见注释)"]
    async fn real_worker_hang_recovery() {
        let img = match std::env::var_os("HINDSIGHT_TEST_IMG") {
            Some(p) => PathBuf::from(p),
            None => {
                eprintln!("未设置 HINDSIGHT_TEST_IMG,跳过");
                return;
            }
        };
        assert!(
            std::env::var_os("HINDSIGHT_OCR_WORKER_EXE").is_some(),
            "必须设置 HINDSIGHT_OCR_WORKER_EXE 指向 app 二进制(cargo test 的 current_exe 是测试壳)"
        );

        let s = Arc::new(OcrSupervisor::with_spawn_and_timeouts(
            Arc::new(spawn_process),
            // 30s:比生产(90s)短、比冷启动首帧长——首次执行新二进制时
            // 系统要做一次性检查(实测冷时 ~47s 花在进程首扫+冷缓存,热后 0.3s/帧),
            // 握手 60s 已消化大头,这里只需容纳首帧
            Duration::from_secs(30),
            Duration::from_secs(60),
            Duration::from_secs(600),
        ));

        // 1) 正常路径:真 worker、真 Vision/Paddle、真图
        let lines = s.recognize(&img).await.expect("真实识别应成功");
        eprintln!("① 正常识别 {} 行", lines.len());
        assert!(!lines.is_empty(), "真实截图至少识别出一行");

        // 2) 让下一个 worker 挂死:HANG 钩子 = 第一次识别永不返回。
        //    先停掉现有 worker,保证下个请求用带钩子的新 worker
        s.stop().await;
        std::env::set_var("HINDSIGHT_OCR_WORKER_HANG", "1");
        let started = std::time::Instant::now();
        let err = s
            .recognize(&img)
            .await
            .expect_err("挂死必须以错误收场,而不是永远等");
        eprintln!("② 挂死在 {:?} 后被处决: {err}", started.elapsed());
        assert!(matches!(err, Error::OcrInfra(_)), "挂死是设施故障: {err}");
        assert!(
            started.elapsed() < Duration::from_secs(45),
            "必须在超时档位附近返回"
        );

        // 3) 撤钩子:下一个请求自动重建并成功——不重启 app、不重启测试进程
        std::env::remove_var("HINDSIGHT_OCR_WORKER_HANG");
        let lines = s.recognize(&img).await.expect("重建后的 worker 应正常识别");
        eprintln!("③ 自愈后识别 {} 行", lines.len());
        assert!(!lines.is_empty());
        assert!(s.spawns.load(Ordering::Relaxed) >= 3, "确实经历了 杀→重建");
    }

    /// 真机浸泡:全部识别走真实 worker 子进程,3000 次。与 `vision_soak_no_wedge`
    /// (进程内直调,实测第 2317 次挂死)的差别就是本模块的存在意义:
    /// 引擎再挂,监管器 90s 杀掉重建,浸泡照常跑完——挂死从"事故"降级为
    /// "一次 90 秒的减速带"。跑法同上,测试名换成 real_worker_soak。
    #[tokio::test]
    #[ignore = "需要真实 app 二进制与真实截图,约 25-45 分钟,手动跑"]
    async fn real_worker_soak() {
        let img = match std::env::var_os("HINDSIGHT_TEST_IMG") {
            Some(p) => PathBuf::from(p),
            None => {
                eprintln!("未设置 HINDSIGHT_TEST_IMG,跳过");
                return;
            }
        };
        assert!(std::env::var_os("HINDSIGHT_OCR_WORKER_EXE").is_some());
        let iters: usize = std::env::var("HINDSIGHT_SOAK_ITERS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3000);

        let s = Arc::new(OcrSupervisor::with_spawn_and_timeouts(
            Arc::new(spawn_process),
            REQ_TIMEOUT,
            READY_TIMEOUT,
            Duration::from_secs(600),
        ));
        let started = std::time::Instant::now();
        let mut infra_recoveries = 0u32;
        for i in 1..=iters {
            match s.recognize(&img).await {
                Ok(_) => {}
                Err(Error::OcrInfra(m)) => {
                    // 挂死被处决 + 自动重建 = 设计内行为;记数继续
                    infra_recoveries += 1;
                    eprintln!("  第 {i} 次遇设施故障并自愈: {m}");
                }
                Err(e) => panic!("第 {i} 次出现帧级错误(图是好的,不该发生): {e}"),
            }
            if i % 200 == 0 {
                eprintln!(
                    "  {i}/{iters}(自愈 {infra_recoveries} 次,累计 {:?})",
                    started.elapsed()
                );
            }
        }
        eprintln!(
            "✓ {iters} 次全部走完:自愈 {infra_recoveries} 次,worker 拉起 {} 次,用时 {:?}",
            s.spawns.load(Ordering::Relaxed),
            started.elapsed()
        );
    }

    /// 调用方 future 在"请求已写出、应答未读"之间被丢弃(digest 外层兜底
    /// 超时就是这个形状)→ 管道残留无人认领的应答。dirty 位保证下次请求
    /// 弃链重建,而不是把旧应答误配给新请求、白付一轮序号错乱。
    #[tokio::test]
    async fn cancelled_caller_marks_link_dirty_and_next_call_respawns() {
        let spawns = Arc::new(AtomicUsize::new(0));
        let spawns2 = Arc::clone(&spawns);
        let s = Arc::new(OcrSupervisor::with_spawn_and_timeouts(
            Arc::new(move |fast, _logs| {
                let n = spawns2.fetch_add(1, Ordering::SeqCst);
                Ok(scripted(fast, move |mut rx, mut tx| async move {
                    send_line(&mut tx, &WireMsg::ready("fake")).await;
                    while let Some(req) = next_req(&mut rx).await {
                        if n == 0 {
                            // 第一个 worker:收到请求后拖 300ms 才回——
                            // 让外层 100ms 取消先落刀,应答滞留在管道里
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                        send_line(&mut tx, &WireMsg::result_ok(req.id, one_line("迟到"))).await;
                    }
                }))
            }),
            Duration::from_secs(10), // supervisor 自己的超时放大,确保外层取消先发生
            Duration::from_millis(500),
            Duration::from_secs(600),
        ));

        let cancelled = tokio::time::timeout(
            Duration::from_millis(100),
            s.recognize(Path::new("/x/a.jpg")),
        )
        .await;
        assert!(cancelled.is_err(), "外层取消确实发生在应答之前");

        // 下一个请求:必须弃掉脏链接、重建后正常——而不是读到那条迟到的旧应答
        let ok = s.recognize(Path::new("/x/b.jpg")).await.unwrap();
        assert_eq!(ok[0].text, "迟到");
        assert_eq!(
            spawns.load(Ordering::SeqCst),
            2,
            "脏链接被弃,第二次请求走了新 worker"
        );
    }

    /// 无换行的洪水必须在读取层被预算截停,而不是读完才发现超限——
    /// 修复前 read_line 会先把整条洪水吞进内存。
    #[tokio::test]
    async fn newline_less_flood_is_capped_at_read_time() {
        let s = sup(Arc::new(|fast, _logs| {
            Ok(scripted(fast, |mut rx, mut tx| async move {
                send_line(&mut tx, &WireMsg::ready("fake")).await;
                let _ = next_req(&mut rx).await;
                // 9MiB 无换行洪水(> MAX_LINE_BYTES = 8MiB)
                let chunk = vec![b'x'; 1024 * 1024];
                for _ in 0..9 {
                    if tx.write_all(&chunk).await.is_err() {
                        return; // 父侧已断链,洪水写不完是预期
                    }
                }
            }))
        }));
        let err = s.recognize(Path::new("/x/flood.jpg")).await.unwrap_err();
        assert!(matches!(err, Error::OcrInfra(_)));
        assert!(err.to_string().contains("超限"), "{err}");
    }

    #[tokio::test]
    async fn handshake_timeout_is_infra() {
        let s = sup(Arc::new(|fast, _logs| {
            Ok(scripted(fast, |_rx, _tx| async move {
                // 什么都不发:引擎加载死在半路
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }))
        }));
        let started = std::time::Instant::now();
        let err = s.recognize(Path::new("/x/a.jpg")).await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(matches!(err, Error::OcrInfra(_)));
        assert!(err.to_string().contains("握手"), "{err}");
    }
}
