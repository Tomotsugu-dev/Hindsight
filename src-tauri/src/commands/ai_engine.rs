//! 引擎 (llama-server) 进程的状态查询、启动、停止、模型切换、日志拉取命令。
//!
//! 引擎的 binary 安装由 [`crate::commands::ai_binary`] 管，这里仅管运行时进程；
//! [`get_engine_status`] 把两者合并一次返回方便前端拉取。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::ai::binary::{self, EngineBinaryStatus};
use crate::ai::job_guard::{self, JobInitState};
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
    /// 当前子进程保护状态：
    /// - `None` —— 保护正常工作
    /// - `Some(reason)` —— 已降级，附中文原因供前端展示
    pub protection_degraded: Option<String>,
}

/// 查询引擎当前状态：binary 是否已安装 + server 是否在跑 + 子进程保护是否正常。
#[tauri::command]
pub async fn get_engine_status(
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<EngineStatusResp, String> {
    let binary = binary::status().map_err(String::from)?;
    let runtime = supervisor.status().await;
    let protection_degraded = match job_guard::init_state() {
        JobInitState::Ok => None,
        JobInitState::NotInitialized => Some("子进程保护未初始化（启动顺序异常）".to_string()),
        JobInitState::Degraded(reason) => Some(reason),
    };
    Ok(EngineStatusResp {
        binary,
        runtime,
        protection_degraded,
    })
}

/// 启动 llama-server 子进程。
///
/// 读 `settings.ai.active_main` / `active_mmproj` 决定加载哪个模型。
/// 没选模型时直接拒绝启动——比让 server 起空跑然后 `/v1/models` 返空
/// 列表友好（用户能立刻知道要先去选）。
#[tauri::command]
pub async fn start_engine(
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<u16, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    if cfg.ai.active_main.trim().is_empty() {
        return Err("请先在「模型」里下载并使用一个模型，再启动引擎".to_string());
    }
    let models_dir = crate::ai::models::root_dir(&cfg.ai);
    let main_path = models_dir.join(&cfg.ai.active_main);
    if !main_path.exists() {
        return Err(format!(
            "选中的主权重不存在：{}（可能被删除或路径变了）",
            cfg.ai.active_main
        ));
    }
    let mmproj_path = if cfg.ai.active_mmproj.trim().is_empty() {
        None
    } else {
        let p = models_dir.join(&cfg.ai.active_mmproj);
        if !p.exists() {
            return Err(format!(
                "选中的 vision 投影文件不存在：{}",
                cfg.ai.active_mmproj
            ));
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

/// 切换 / 设置当前在用的模型。
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

/// 拿 llama-server 子进程最近 N 行 stderr/stdout（ring buffer 最大 500 行）。
/// 调试 tab 用：看 GPU 加载日志（`offloaded XX/YY layers to GPU` / `cuBLAS init` 等）。
#[tauri::command]
pub async fn get_engine_logs(
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<Vec<String>, String> {
    Ok(supervisor.recent_logs().await)
}
