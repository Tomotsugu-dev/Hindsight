//! 单个 `llama-server` 子进程的拉起、存活检测、终止与失败诊断。

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Child;

use super::command::build_command;
use super::logs::EngineLogs;
use super::{EngineStartOverrides, HEALTH_TIMEOUT};

/// 等待子进程退出回收的上限时长，避免进程卡死在不可中断状态导致上层调用挂死。
const SHUTDOWN_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

/// 运行中引擎的绑定元信息（监听端口、模型路径与启动参数）。
#[derive(Debug, Clone)]
pub(super) struct LoadedEngine {
    pub port: u16,
    pub model: PathBuf,
    pub params: EngineStartOverrides,
}

/// 已拉起的 `llama-server` 子进程句柄及其绑定信息。
pub(super) struct LlamaServer {
    pub(super) child: Child,
    pub(super) loaded: LoadedEngine,
}

impl LlamaServer {
    /// 挑选空闲本地端口并拉起加载 `model` 的 `llama-server`（不等待 `/health` 就绪）。
    ///
    /// 启动前会清空 `logs` 缓冲，并为子进程的 `stderr` / `stdout` 启动后台收集任务。
    pub(super) async fn spawn(
        binary_path: &Path,
        model: PathBuf,
        overrides: EngineStartOverrides,
        logs: &EngineLogs,
    ) -> std::io::Result<Self> {
        let port = pick_free_port()?;
        let mut cmd = build_command(binary_path, port, &model, &overrides);
        // 挂载平台相关的 spawn 前子进程保护钩子（Linux/macOS 设 setpgid / PDEATHSIG；Windows 为 no-op）。
        if let Err(err) = crate::ai::job_guard::prepare_command(&mut cmd) {
            log::warn!("job_guard::prepare_command 失败（保护可能降级）: {err}");
        }
        // 记录关键启动参数与模型文件名，便于从日志中排查 -np / --batch-size 及模型切换问题。
        let model_name = model
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<none>".to_string());
        log::info!(
            "[engine-spawn] port={port} main={model_name} batch={:?} -np={:?} ctx_per_slot={:?}",
            overrides.batch_size,
            overrides.parallel_slots,
            overrides.ctx_size,
        );

        let mut child = cmd.spawn()?;

        // Windows 上将子进程加入全局 Job Object，确保父进程异常崩溃时由内核一并清理；
        // Linux / macOS 上为空操作。失败仅记录警告，不阻断引擎启动。
        if let Some(pid) = child.id() {
            if let Err(err) = crate::ai::job_guard::assign_child_pid(pid) {
                log::warn!("AssignProcessToJobObject pid={pid} 失败: {err}");
            }
        }

        logs.clear().await;
        if let Some(stderr) = child.stderr.take() {
            logs.spawn_reader(stderr, "stderr");
        }
        if let Some(stdout) = child.stdout.take() {
            logs.spawn_reader(stdout, "stdout");
        }

        Ok(Self {
            child,
            loaded: LoadedEngine {
                port,
                model,
                params: overrides,
            },
        })
    }

    /// 返回子进程是否已经退出（非阻塞检查）。
    pub(super) fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// 终止子进程并在 [`SHUTDOWN_WAIT_TIMEOUT`] 内等待回收。
    pub(super) async fn kill(mut self) {
        let _ = self.child.kill().await;
        wait_child_with_timeout(&mut self.child).await;
    }

    /// 启动未就绪时调用：若进程仍存活则强制终止，并从日志末尾整理可读失败原因。
    pub(super) async fn failure_reason(mut self, logs: &EngineLogs) -> String {
        let exit_status = self.child.try_wait();
        if matches!(exit_status, Ok(None)) {
            let _ = self.child.kill().await;
            wait_child_with_timeout(&mut self.child).await;
            // 给日志读取任务留出短暂时间刷完残余 stderr 输出。
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
            Err(err) => format!("无法读取子进程状态：{err}"),
        }
    }
}

/// 绑定 `127.0.0.1:0` 让系统分配一个当前空闲端口后立即释放。
/// 释放到子进程真正 bind 之间存在极短竞态窗口，若撞车会由后续启动失败逻辑捕获。
fn pick_free_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

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
