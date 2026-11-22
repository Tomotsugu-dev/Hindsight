//! 一个 llama-server 子进程：拉起、关掉，没起来时说原因。

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Child;

use super::command::build_command;
use super::logs::Logs;
use super::{EngineStartOverrides, HEALTH_TIMEOUT};

/// 跑着的引擎装着什么。
#[derive(Debug, Clone)]
pub(super) struct Loaded {
    pub port: u16,
    pub model: PathBuf,
    pub params: EngineStartOverrides,
}

/// 拉起来的 llama-server 进程。
pub(super) struct LlamaServer {
    pub(super) child: Child,
    pub(super) loaded: Loaded,
}

impl LlamaServer {
    /// 在空闲端口上用 `bin` 拉起装着 `model` 的引擎，不等它能用。
    ///
    /// 输出记进 `logs`，上一次的先清掉。
    pub(super) async fn spawn(
        bin: &Path,
        model: PathBuf,
        params: EngineStartOverrides,
        logs: &Logs,
    ) -> std::io::Result<Self> {
        let port = pick_free_port()?;
        let mut cmd = build_command(bin, port, &model, &params);
        // 装平台特定的 spawn 前钩子（Linux/macOS 设 setpgid + PDEATHSIG）；
        // Windows 是 no-op，post-spawn 才装 Job
        if let Err(e) = crate::ai::job_guard::prepare_command(&mut cmd) {
            log::warn!("job_guard::prepare_command 失败（保护可能降级）: {e}");
        }
        // 调试用：把最终拼出的命令行打到 log，方便排查 -np / --batch-size 是否生效，
        // 同时打模型文件名，确认日报、周报、聊天之间切换时加载的是哪个模型。
        // 标记 [engine-spawn] 让用户在长 log 里 grep 能直接定位。
        let main_name = model
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<none>".to_string());
        log::info!(
            "[engine-spawn] port={port} main={main_name} batch={:?} -np={:?} ctx_per_slot={:?}",
            params.batch_size,
            params.parallel_slots,
            params.ctx_size,
        );

        let mut child = cmd.spawn()?;

        // 把 child 加到全局 Job Object —— Hindsight 进程死时 OS 内核会同步杀光
        // 所有 Job 成员，无视父进程怎么死（panic / Ctrl+C / taskkill）。
        // assign 失败不阻塞启动，只 log 警告退化到原 Exit hook 路径。
        // Linux / macOS 上 assign_child_pid 是 no-op。
        if let Some(pid) = child.id() {
            if let Err(e) = crate::ai::job_guard::assign_child_pid(pid) {
                log::warn!("AssignProcessToJobObject pid={pid} 失败: {e}");
            }
        }

        // 启动前清空日志缓冲，让本次启动从空白开始
        logs.clear().await;
        if let Some(stderr) = child.stderr.take() {
            logs.collect(stderr, "stderr");
        }
        if let Some(stdout) = child.stdout.take() {
            logs.collect(stdout, "stdout");
        }

        Ok(Self {
            child,
            loaded: Loaded {
                port,
                model,
                params,
            },
        })
    }

    /// 进程自己退出了。
    pub(super) fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// 关掉进程，限时等它退出。
    pub(super) async fn kill(mut self) {
        let _ = self.child.kill().await;
        wait_child_with_timeout(&mut self.child).await;
    }

    /// 没等到它能用时调：还活着就关掉，再从日志末尾拿原因给前端看。
    pub(super) async fn failure_reason(mut self, logs: &Logs) -> String {
        let exit_status = self.child.try_wait();
        if matches!(exit_status, Ok(None)) {
            // 还活着但 hang——强制 kill
            let _ = self.child.kill().await;
            wait_child_with_timeout(&mut self.child).await;
            // 等 50ms 让 drain task 把残余 stderr 行写完
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let tail = logs.tail(12).await;
        match exit_status {
            Ok(Some(_)) => {
                if tail.is_empty() {
                    "子进程已退出但无日志输出".to_string()
                } else {
                    format!("子进程退出；日志末尾：\n{tail}")
                }
            }
            Ok(None) => {
                if tail.is_empty() {
                    format!(
                        "启动 {}s 未响应 /health，已强制 kill",
                        HEALTH_TIMEOUT.as_secs()
                    )
                } else {
                    format!(
                        "启动 {}s 未响应 /health，已强制 kill；日志末尾：\n{tail}",
                        HEALTH_TIMEOUT.as_secs()
                    )
                }
            }
            Err(e) => format!("无法读取子进程状态：{e}"),
        }
    }
}

/// 让 OS 挑一个未占用的端口然后立刻释放——子进程 spawn 用这个端口。
///
/// drop 与 spawn 之间有 race window（~ms 级），其它进程可能抢到这个端口，
/// 但概率极低。如果真撞上，llama-server 启动会 bind 失败，被我们的错误捕获处理。
fn pick_free_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// 限时等子进程 wait()，避免子进程卡住把 idle watcher / stop() 调用栈吊死。
///
/// kill 信号已发送到内核层；child.wait() 主要等内核回收 PID，正常 ms 级返回。
/// 超过 [`SHUTDOWN_WAIT_TIMEOUT`] 还没回 = 进程被 D 状态卡死或者僵尸表满了，
/// 此时再等下去也意义不大，放弃等待让上层流程往下走。
const SHUTDOWN_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

async fn wait_child_with_timeout(child: &mut Child) {
    if tokio::time::timeout(SHUTDOWN_WAIT_TIMEOUT, child.wait())
        .await
        .is_err()
    {
        log::warn!(
            "llama-server wait 超过 {}s 仍未退出，放弃等待（OS 应已收到 kill 信号自行清理）",
            SHUTDOWN_WAIT_TIMEOUT.as_secs()
        );
    }
}
