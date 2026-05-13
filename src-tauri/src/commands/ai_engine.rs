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
use crate::ai::platform::{self, BackendCapabilities, BackendChoice, VramInfo};
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
    /// 由 [`platform::detect_total_vram_gb`] 提供,OnceLock 全局缓存,
    /// 每次轮询都直接命中缓存（首次约 100-500ms）。
    pub system_vram: Option<VramInfo>,
    /// 系统三档 backend 的可用性（`{cuda, vulkan, cpu}`）——给前端 EngineTab
    /// 的下拉框判断哪些选项要灰掉。macOS / Linux 上三档全 false 即可，
    /// 前端那边只在 Windows 路由下渲染 backend 下拉。
    pub backend_capabilities: BackendCapabilities,
    /// 当前用户选择的 backend 偏好,前端下拉框回显选中态用。
    /// 取值跟 [`BackendChoice::as_str`] 一致："auto" / "cuda" / "vulkan" / "cpu"。
    pub backend_choice: String,
}

/// 查询引擎当前状态：binary 是否已安装 + onnxruntime 是否已安装 +
/// server 是否在跑 + 子进程保护是否正常 + 系统 VRAM + 三档 backend 可用性 + 用户偏好。
#[tauri::command]
pub async fn get_engine_status(
    pool: State<'_, DbPool>,
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
    let backend_capabilities = platform::detect_backend_capabilities();
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    Ok(EngineStatusResp {
        binary,
        runtime,
        embedding_runtime,
        protection_degraded,
        system_vram,
        backend_capabilities,
        backend_choice: cfg.ai.backend_choice,
    })
}

/// 切换 backend 偏好——写 settings + 同步到 platform 模块全局原子状态。
///
/// 写完之后下次任何 `platform::detect()` 调用都会反映新偏好，但已经下载到磁盘的
/// 旧 backend binary **不会自动删**（每个 backend 有独立 platform_dir，多份共存不冲突，
/// 切回去零等待）。前端 EngineTab 在确认弹窗里负责串联：
/// `set_backend_choice → 按需 download_binary`。
///
/// `choice` 接受 "auto" / "cuda" / "vulkan" / "cpu"；未知值由 [`BackendChoice::from_str`]
/// 静默回退到 "auto"。
#[tauri::command]
pub async fn set_backend_choice(
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    choice: String,
) -> Result<(), String> {
    // 命令体自己 sanitize：`settings::save` 不调 ai::config::sanitize（那条路径只有
    // `update_settings` 命令走），所以这里如果直接落原始入参，DB 会真的存 "garbage"
    // 之类非法值，前端 SimplePicker 找不到匹配 option 就显示空 label。
    // 这里把入参先经 [`BackendChoice::from_str`] 钳到合法枚举值再 [`as_str`] 拿回字符串落库。
    let parsed = BackendChoice::from_str(&choice);
    let mut cfg = settings::load(&pool).await.map_err(String::from)?;
    cfg.ai.backend_choice = parsed.as_str().to_string();
    settings::save(&pool, &cfg).await.map_err(String::from)?;

    // 同步到 platform 模块的全局原子状态——下次 `platform::detect()` 立刻反映新偏好
    platform::set_user_preference(parsed);

    // backend 换了 = 当前在跑的 server 跑的是旧 binary，停掉等用户主动重启
    let _ = supervisor.stop().await;
    Ok(())
}

/// 启动 llama-server 子进程。
///
/// 加载 step 1（图描述）的 vision 模型——`effective_describe_main()` 优先读
/// `describe_main`，空则 fallback 到老的 `active_main`。手动启动按钮一般是给
/// daily / debug 跑前预热引擎用，加载 vision 模型最通用（既能跑 step 1 又能
/// 跑 step 2 纯文本任务）。
///
/// 没选任何模型时直接拒绝启动——比让 server 起空跑然后 `/v1/models` 返空
/// 列表友好（用户能立刻知道要先去选）。
#[tauri::command]
pub async fn start_engine(
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<u16, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    let main_name = cfg.ai.effective_describe_main();
    if main_name.trim().is_empty() {
        return Err("请先在「模型」里给图描述（step 1）选一个模型，再启动引擎".to_string());
    }
    let models_dir = crate::ai::models::root_dir(&cfg.ai);
    let main_path = models_dir.join(main_name);
    if !main_path.exists() {
        return Err(format!(
            "选中的主权重不存在：{main_name}（可能被删除或路径变了）"
        ));
    }
    let mmproj_name = cfg.ai.effective_describe_mmproj();
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

/// 单独设置 step 1（图描述）或 step 2（段总结）的模型；其它 step 不动。
///
/// `step` 取 `"describe"` / `"summary"`；`main_file` 空字符串 = 清掉该 step 的覆盖
/// （随后该 step fallback 到 `active_main`）。同时 stop 在跑的 server，下次跑总结
/// 时按新模型 lazy spawn。
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
        "describe" => {
            cfg.ai.describe_main = main;
            cfg.ai.describe_mmproj = mmproj;
        }
        "summary" => {
            cfg.ai.summary_main = main;
            cfg.ai.summary_mmproj = mmproj;
        }
        other => {
            return Err(format!(
                "set_step_model: 未知 step {other}（仅支持 describe / summary）"
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
