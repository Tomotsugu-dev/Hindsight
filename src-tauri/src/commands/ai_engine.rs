//! 引擎 (llama-server) 进程的状态查询、启动、停止、模型切换、日志拉取命令。
//!
//! 引擎的 binary 安装由 [`crate::commands::ai_binary`] 管，这里仅管运行时进程；
//! [`get_engine_status`] 把两者合并一次返回方便前端拉取。

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::ai::binary::{self, EngineBinaryStatus};
use crate::ai::config::{
    sanitize as sanitize_ai, AiConfig, ModelRef, Pick, Task, CLOUD_UNAVAILABLE_ERROR,
    NO_MODEL_ERROR,
};
use crate::ai::embedding_runtime::{self, RuntimeStatus};
use crate::ai::engine::{EngineRuntimeStatus, EngineStartOverrides, EngineSupervisor};
use crate::ai::job_guard::{self, JobInitState};
use crate::ai::platform::{self, VramInfo};
use crate::commands::ai_endpoint::{self, test_ai_chat, test_ai_endpoint};
use crate::commands::ai_summary::{RunLock, AI_RUN_BUSY};
use crate::repo::settings;
use crate::storage::DbPool;

/// 引擎状态合并响应：binary 安装状态 + server 运行时状态 + 子进程保护状态。
///
/// 前端拿一次拉到全套，不用串行调两个命令；`protection_degraded` 让 UI
/// 能在 macOS / 异常 Job 初始化等场景给用户提示"保护已降级"。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatusResp {
    #[serde(flatten)]
    pub binary: EngineBinaryStatus,
    pub runtime: EngineRuntimeStatus,
    /// onnxruntime 推理库（embedding 用）安装状态。跟 `binary` 是 AI 引擎的两半，
    /// 任一未装都视作"AI 引擎未就绪"——`download_binary` 一次性下两份。
    pub embedding_runtime: RuntimeStatus,
    /// 当前子进程保护状态：
    /// - `None` —— 保护正常工作
    /// - `Some(reason)` —— 已降级，附中文原因供前端展示
    pub protection_degraded: Option<String>,
    /// 系统 VRAM 信息（NVIDIA discrete 或 Apple Silicon unified × 0.7）。
    /// `None` = CPU-only 机器或探测失败——前端按"未检测到独立显存"处理。
    /// 由 [`platform::detect_total_vram_gb`] 提供，OnceLock 全局缓存，
    /// 每次轮询都直接命中缓存（首次约 100-500ms）。
    pub system_vram: Option<VramInfo>,
}

/// 查询引擎当前状态：binary 是否已安装 + onnxruntime 是否已安装 +
/// server 是否在跑 + 子进程保护是否正常 + 系统 VRAM。
#[tauri::command]
pub async fn get_engine_status(
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<EngineStatusResp, String> {
    let binary = binary::status().map_err(String::from)?;
    let embedding_runtime = embedding_runtime::status().map_err(String::from)?;
    let runtime = supervisor.status().await;
    let protection_degraded = match job_guard::init_state() {
        JobInitState::Ok => None,
        JobInitState::NotInitialized => Some("子进程保护未初始化（启动顺序异常）".to_string()),
        JobInitState::Degraded(reason) => Some(reason),
    };
    let system_vram = platform::detect_total_vram_gb();
    Ok(EngineStatusResp {
        binary,
        runtime,
        embedding_runtime,
        protection_degraded,
        system_vram,
    })
}

/// 单个任务的 AI 连接探测结果。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskConnectionTest {
    pub task: Task,
    /// 连接类型：`"local"`、`"cloud"` 或 `"unconfigured"`。
    pub kind: &'static str,
    pub model: String,
    pub ok: bool,
    pub message: String,
}

fn overrides_for_task(ai: &AiConfig, task: Task) -> EngineStartOverrides {
    match task {
        Task::Chat => EngineStartOverrides {
            batch_size: None,
            parallel_slots: None,
            ctx_size: ai.chat_ctx_size(),
        },
        Task::Daily | Task::Weekly => EngineStartOverrides {
            batch_size: ai.batch_size_for(task),
            parallel_slots: ai.parallel_slots_for(task),
            ctx_size: ai.ctx_size_for(task),
        },
    }
}

/// 探测日报、周报、对话三个任务当前配置的 AI 连接。
///
/// 本地模型会按各自任务的高级参数启动，测试完成后没人在用就停止，释放显存；
/// 若相邻任务共用相同的本地模型与启动参数，则复用同一次启动。
/// 日报、周报或聊天正在用引擎时，本地模型那一项不测，报忙。
#[tauri::command]
pub async fn test_ai_connections(
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    run_lock: State<'_, RunLock>,
) -> Result<Vec<TaskConnectionTest>, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    let ai = &cfg.ai;
    let models_dir = crate::ai::models::root_dir(ai);
    // 拿不到锁 = 日报或周报正在跑。拿到了就一直拿着，测试期间它们不会启动
    let run_guard = run_lock.0.try_lock().ok();

    let mut results = Vec::with_capacity(3);
    let mut started_local = false;
    let mut last_local: Option<(PathBuf, EngineStartOverrides, u16)> = None;

    for task in [Task::Daily, Task::Weekly, Task::Chat] {
        match ai.model_for(task) {
            None => {
                results.push(TaskConnectionTest {
                    task,
                    kind: "unconfigured",
                    model: String::new(),
                    ok: false,
                    message: NO_MODEL_ERROR.to_string(),
                });
            }
            Some(ModelRef::Cloud { connection }) => {
                let Some(conn) = ai.cloud_connection(&connection).filter(|c| c.is_ready()) else {
                    results.push(TaskConnectionTest {
                        task,
                        kind: "cloud",
                        model: connection,
                        ok: false,
                        message: CLOUD_UNAVAILABLE_ERROR.to_string(),
                    });
                    continue;
                };
                let key = (!conn.api_key.is_empty()).then(|| conn.api_key.clone());
                let endpoint_resp = test_ai_endpoint(conn.endpoint.clone(), key.clone())
                    .await
                    .unwrap_or_else(|e| ai_endpoint::fail(&e));
                if !endpoint_resp.ok {
                    results.push(TaskConnectionTest {
                        task,
                        kind: "cloud",
                        model: conn.model.clone(),
                        ok: false,
                        message: endpoint_resp.message,
                    });
                    continue;
                }
                let chat_resp = test_ai_chat(conn.endpoint.clone(), key, conn.model.clone(), false)
                    .await
                    .unwrap_or_else(|e| ai_endpoint::fail(&e));
                results.push(TaskConnectionTest {
                    task,
                    kind: "cloud",
                    model: conn.model.clone(),
                    ok: chat_resp.ok,
                    message: chat_resp.message,
                });
            }
            Some(ModelRef::Local { file }) => {
                let main_path = models_dir.join(&file);
                if !main_path.exists() {
                    results.push(TaskConnectionTest {
                        task,
                        kind: "local",
                        model: file.clone(),
                        ok: false,
                        message: format!(
                            "[AI_MODEL_MISSING] Selected local model file not found: {file}"
                        ),
                    });
                    continue;
                }
                let overrides = overrides_for_task(ai, task);
                let port = match &last_local {
                    Some((prev_path, prev_ov, prev_port))
                        if *prev_path == main_path && *prev_ov == overrides =>
                    {
                        Ok(*prev_port)
                    }
                    _ => {
                        // 要重启引擎时，别的任务正在用就不动它，这一项报忙
                        if run_guard.is_none() || supervisor.in_use().await {
                            results.push(TaskConnectionTest {
                                task,
                                kind: "local",
                                model: file,
                                ok: false,
                                message: AI_RUN_BUSY.to_string(),
                            });
                            continue;
                        }
                        started_local = true;
                        match supervisor
                            .restart_with_overrides(
                                Some(main_path.clone()),
                                None,
                                overrides.clone(),
                            )
                            .await
                        {
                            Ok(p) => {
                                last_local = Some((main_path, overrides, p));
                                Ok(p)
                            }
                            Err(e) => {
                                last_local = None;
                                Err(e.to_string())
                            }
                        }
                    }
                };
                match port {
                    Ok(p) => {
                        let resp = test_ai_endpoint(format!("http://127.0.0.1:{p}/v1"), None)
                            .await
                            .unwrap_or_else(|e| ai_endpoint::fail(&e));
                        results.push(TaskConnectionTest {
                            task,
                            kind: "local",
                            model: file,
                            ok: resp.ok,
                            message: resp.message,
                        });
                    }
                    Err(msg) => {
                        results.push(TaskConnectionTest {
                            task,
                            kind: "local",
                            model: file,
                            ok: false,
                            message: msg,
                        });
                    }
                }
            }
        }
    }

    // 测试期间聊天可能已经用上这个引擎。有人在用就不停，交给空闲回收
    if started_local && !supervisor.in_use().await {
        let _ = supervisor.stop().await;
    }

    Ok(results)
}

/// 停止当前正在运行的 llama-server 子进程（如有）。
#[tauri::command]
pub async fn stop_engine(supervisor: State<'_, Arc<EngineSupervisor>>) -> Result<(), String> {
    supervisor.stop().await.map_err(String::from)
}

/// 设置或清空指定 AI 任务的模型选择，并停止当前运行中的本地服务。
#[tauri::command]
pub async fn set_task_model(
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    task: Task,
    model: Option<Pick>,
) -> Result<(), String> {
    let mut cfg = settings::load(&pool).await.map_err(String::from)?;
    let old_ai = cfg.ai.clone();
    match task {
        Task::Daily => {
            cfg.ai.picks.daily = match model {
                Some(Pick::Local { file }) => Some(ModelRef::Local { file }),
                Some(Pick::Cloud { connection }) => Some(ModelRef::Cloud { connection }),
                Some(Pick::Inherit) | None => None,
            };
        }
        Task::Weekly => {
            cfg.ai.picks.weekly = model.unwrap_or(Pick::Inherit);
        }
        Task::Chat => {
            cfg.ai.picks.chat = match model {
                Some(Pick::Local { file }) => Some(ModelRef::Local { file }),
                Some(Pick::Cloud { connection }) => Some(ModelRef::Cloud { connection }),
                Some(Pick::Inherit) | None => None,
            };
        }
    }
    cfg.ai = sanitize_ai(cfg.ai, &old_ai);
    settings::save(&pool, &cfg).await.map_err(String::from)?;
    let _ = supervisor.stop().await;
    Ok(())
}
