//! 引擎 (llama-server) 进程的状态查询、启动、停止、模型切换、日志拉取命令。
//!
//! 引擎的 binary 安装由 [`crate::commands::ai_binary`] 管，这里仅管运行时进程；
//! [`get_engine_status`] 把两者合并一次返回方便前端拉取。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::ai::binary::{self, EngineBinaryStatus};
use crate::ai::embedding_runtime::{self, RuntimeStatus};
use crate::ai::job_guard::{self, JobInitState};
use crate::ai::platform::{self, VramInfo};
use crate::ai::server::{EngineRuntimeStatus, EngineSupervisor};
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

/// 启动 llama-server 子进程。
///
/// 加载段总结的文本模型——`effective_summary_main()` 优先读 `summary_main`，
/// 空则 fallback 到老的 `active_main`。手动启动按钮一般是给跑总结/对话前
/// 预热引擎用。
///
/// 没选任何模型时直接拒绝启动——比让 server 起空跑然后 `/v1/models` 返空
/// 列表友好（用户能立刻知道要先去选）。
#[tauri::command]
pub async fn start_engine(
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<u16, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    let main_name = cfg.ai.effective_summary_main();
    if main_name.trim().is_empty() {
        return Err("请先在「模型」里选一个模型，再启动引擎".to_string());
    }
    let models_dir = crate::ai::models::root_dir(&cfg.ai);
    let main_path = models_dir.join(main_name);
    if !main_path.exists() {
        return Err(format!(
            "选中的主权重不存在：{main_name}（可能被删除或路径变了）"
        ));
    }
    let mmproj_name = cfg.ai.effective_summary_mmproj();
    let mmproj_path = if mmproj_name.trim().is_empty() {
        None
    } else {
        let p = models_dir.join(mmproj_name);
        if !p.exists() {
            return Err(format!("选中的 vision 投影文件不存在：{mmproj_name}"));
        }
        Some(p)
    };
    supervisor
        .start(Some(main_path), mmproj_path)
        .await
        .map_err(String::from)
}

/// 停掉子进程（如果在跑）。
#[tauri::command]
pub async fn stop_engine(supervisor: State<'_, Arc<EngineSupervisor>>) -> Result<(), String> {
    supervisor.stop().await.map_err(String::from)
}

/// 切换 / 设置当前在用的模型（旧版单一字段；新代码请用 [`set_step_model`]）。
///
/// 写 settings 后顺手 stop 在跑的 server——下次用户点"启动引擎"会带新模型重起。
/// 不在这里自动 start，因为 start 可能 90s 才返回，命令调用方等不动；让用户主动触发更可控。
#[tauri::command]
pub async fn set_active_model(
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    main_file: String,
    mmproj_file: Option<String>,
) -> Result<(), String> {
    let mut cfg = settings::load(&pool).await.map_err(String::from)?;
    cfg.ai.active_main = main_file.trim().to_string();
    cfg.ai.active_mmproj = mmproj_file.unwrap_or_default().trim().to_string();
    settings::save(&pool, &cfg).await.map_err(String::from)?;

    // 切了模型，旧 server 跑的就是旧模型，停掉等用户手动重启
    let _ = supervisor.stop().await;
    Ok(())
}

/// 单独设置段总结（summary）/ 对话（chat）的模型；另一个槽位不动。
///
/// `step` 取 `"summary"` / `"chat"`；`main_file` 空字符串 = 清掉该槽位的覆盖
/// （summary fallback 到 `active_main`；chat 回到"自动"路由）。
/// 同时 stop 在跑的 server，下次使用时按新模型 lazy spawn。
/// chat 是纯文本任务不带 mmproj，`mmproj_file` 对它忽略。
#[tauri::command]
pub async fn set_step_model(
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    step: String,
    main_file: String,
    mmproj_file: Option<String>,
) -> Result<(), String> {
    let mut cfg = settings::load(&pool).await.map_err(String::from)?;
    let main = main_file.trim().to_string();
    let mmproj = mmproj_file.unwrap_or_default().trim().to_string();
    match step.as_str() {
        "summary" => {
            cfg.ai.summary_main = main;
            cfg.ai.summary_mmproj = mmproj;
        }
        "chat" => {
            cfg.ai.chat_main = main;
        }
        other => {
            return Err(format!(
                "set_step_model: 未知 step {other}（仅支持 summary / chat）"
            ));
        }
    }
    settings::save(&pool, &cfg).await.map_err(String::from)?;
    let _ = supervisor.stop().await;
    Ok(())
}

/// 拿 llama-server 子进程最近 N 行 stderr/stdout（ring buffer 最大 500 行）。
/// 调试 tab 用：看 GPU 加载日志（`offloaded XX/YY layers to GPU` / `cuBLAS init` 等）。
#[tauri::command]
pub async fn get_engine_logs(
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<Vec<String>, String> {
    Ok(supervisor.recent_logs().await)
}
