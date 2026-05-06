//! AI 子系统的 Tauri 命令薄壳。
//!
//! 业务逻辑（如真正的 LLM 调用）会在 `crate::ai` 模块里实现，
//! 这里只做参数校验 / 错误归类 / 返回对前端友好的形状。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::ai::binary::{self, DownloadPhase, EngineBinaryStatus};
use crate::ai::models::{self, ModelEntry};
use crate::ai::recommended::{Recommended, RECOMMENDED};
use crate::ai::server::{EngineRuntimeStatus, EngineSupervisor};
use crate::ai::summary::{DaySummaryRunner, SummaryProgress, SUMMARY_PROGRESS_EVENT};
use crate::repo::ai_summaries::{self, SegmentSummaryRow};
use crate::repo::reports::device_filter_from_option;
use crate::repo::settings;
use crate::storage::DbPool;

/// AI 总结流程的取消信号——管 lib.rs `manage` 的全局单例。
///
/// 同一时刻只允许一个 generate_day_summary 在跑（前端 UI 不让重复点）；
/// cancel_day_summary 把内部 AtomicBool 设 true，summary.rs 在每段循环检测到后
/// 就停下来 Ok(()) 退出（不能中断已经在路上的单段 LLM 请求）。
pub struct SummaryCancel(pub Arc<AtomicBool>);

impl Default for SummaryCancel {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}

/// `test_ai_endpoint` 的返回。
///
/// 任何失败（网络不通、状态码非 2xx、JSON 解析失败、…）都用 `ok=false + message`
/// 表达，**不抛 Err**——这样前端只需要检查一个布尔字段，不用同时处理 invoke
/// 自身的拒绝路径和返回值的成败两套逻辑。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestAiEndpointResp {
    pub ok: bool,
    /// 成功时取响应里的 `data[].id`，最多前 10 个；失败时为空
    pub models: Vec<String>,
    /// 失败时填给用户看的错误描述；成功时为空
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResp {
    data: Vec<OpenAIModelEntry>,
}

/// OpenAI `/v1/models` 响应里的一项；只为本地解析用，跟
/// 本地磁盘上的 [`crate::ai::models::ModelEntry`] 是两个概念
#[derive(Debug, Deserialize)]
struct OpenAIModelEntry {
    id: String,
}

/// 测试 OpenAI 兼容端点的连通性：`GET {endpoint}/models`。
///
/// `endpoint` 末尾的 `/` 会被吃掉再拼路径，避免出现 `//models`。
/// `api_key` 非空时走 Bearer auth；Ollama 一般不用填。
#[tauri::command]
pub async fn test_ai_endpoint(
    endpoint: String,
    api_key: Option<String>,
) -> Result<TestAiEndpointResp, String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(fail("服务地址为空"));
    }
    let url = format!("{}/models", trimmed);

    let client = match Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Ok(fail(&format!("HTTP 客户端构造失败：{e}"))),
    };

    let mut req = client.get(&url);
    if let Some(k) = api_key.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        req = req.bearer_auth(k);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return Ok(fail(&fmt_send_err(e))),
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        // 截短服务返回，避免把整个 HTML 错误页粘进 toast
        let preview: String = body.chars().take(120).collect();
        return Ok(fail(&format!("服务返回 {status}：{preview}")));
    }

    let parsed: ModelsResp = match resp.json().await {
        Ok(p) => p,
        Err(e) => return Ok(fail(&format!("响应不是 OpenAI 兼容格式：{e}"))),
    };

    let models: Vec<String> = parsed
        .data
        .into_iter()
        .map(|m| m.id)
        .take(10)
        .collect();

    Ok(TestAiEndpointResp {
        ok: true,
        models,
        message: String::new(),
    })
}

/// 把 reqwest 发送错误格式化成对用户可读的字符串。
///
/// reqwest::Error 的 Display 只给最外层（"error sending request for url ..."），
/// 真正告诉用户"为啥连不上"的是 source chain 最深处的 io::Error
/// （`Connection refused`、`os error 10061` 等）。这里把整条链拼起来，
/// 并按 reqwest 提供的分类给一句开场白。
pub(crate) fn fmt_send_err(e: reqwest::Error) -> String {
    let head = if e.is_timeout() {
        "请求超时"
    } else if e.is_connect() {
        "连接失败（确认服务是否启动、端口是否正确）"
    } else if e.is_request() {
        "请求构造失败"
    } else {
        "网络错误"
    };

    // 跳过 reqwest::Error 自己（它的 to_string 跟 URL 重复了），从 source 起拼
    let mut details: Vec<String> = Vec::new();
    let mut cur: Option<&dyn std::error::Error> = std::error::Error::source(&e);
    while let Some(s) = cur {
        details.push(s.to_string());
        cur = s.source();
    }

    if details.is_empty() {
        head.to_string()
    } else {
        format!("{head}：{}", details.join(" → "))
    }
}

fn fail(msg: &str) -> TestAiEndpointResp {
    TestAiEndpointResp {
        ok: false,
        models: Vec::new(),
        message: msg.to_string(),
    }
}

// ─────────────────────────────────────────────────────────
//  本地 llama-server binary 安装命令（Phase 1B-α）
// ─────────────────────────────────────────────────────────

/// 引擎状态合并响应：binary 安装状态 + server 运行时状态。
///
/// 前端拿一次拉到全套，不用串行调两个命令。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatusResp {
    #[serde(flatten)]
    pub binary: EngineBinaryStatus,
    pub runtime: EngineRuntimeStatus,
}

/// 查询引擎当前状态：binary 是否已安装 + server 是否在跑。
#[tauri::command]
pub async fn get_engine_status(
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<EngineStatusResp, String> {
    let binary = binary::status().map_err(|e| e.to_string())?;
    let runtime = supervisor.status().await;
    Ok(EngineStatusResp { binary, runtime })
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
    let cfg = settings::load(&pool).await.map_err(|e| e.to_string())?;
    if cfg.ai.active_main.trim().is_empty() {
        return Err(
            "请先在「模型」里下载并使用一个模型，再启动引擎".to_string(),
        );
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
        .map_err(Into::into)
}

/// 停掉子进程（如果在跑）。
#[tauri::command]
pub async fn stop_engine(
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<(), String> {
    supervisor.stop().await.map_err(Into::into)
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
    let mut cfg = settings::load(&pool).await.map_err(|e| e.to_string())?;
    cfg.ai.active_main = main_file.trim().to_string();
    cfg.ai.active_mmproj = mmproj_file.unwrap_or_default().trim().to_string();
    settings::save(&pool, &cfg).await.map_err(|e| e.to_string())?;

    // 切了模型，旧 server 跑的就是旧模型，停掉等用户手动重启
    let _ = supervisor.stop().await;
    Ok(())
}

// ─────────────────────────────────────────────────────────
//  模型管理（Phase 1B-β β.1）
// ─────────────────────────────────────────────────────────

/// 列当前 settings 配的模型目录里所有 `.gguf` 文件。
/// 目录不存在或为空都返回 `[]`，不当错误。
#[tauri::command]
pub async fn list_local_models(
    pool: State<'_, DbPool>,
) -> Result<Vec<ModelEntry>, String> {
    let cfg = settings::load(&pool).await.map_err(|e| e.to_string())?;
    models::list_local(&cfg.ai).map_err(Into::into)
}

/// 删除一个本地 GGUF 文件。`filename` 必须是文件名（不含路径），后端
/// 会再次校验防 `..` / 分隔符注入。
///
/// 顺手清理：被删文件如果是当前 active_main / active_mmproj，把 settings
/// 里那项清掉——下次 `start_engine` 才不会拿一个不存在的文件名报错。
#[tauri::command]
pub async fn delete_model(
    pool: State<'_, DbPool>,
    filename: String,
) -> Result<(), String> {
    let mut cfg = settings::load(&pool).await.map_err(|e| e.to_string())?;
    models::delete(&cfg.ai, &filename).map_err(|e| e.to_string())?;

    let mut dirty = false;
    if cfg.ai.active_main == filename {
        cfg.ai.active_main.clear();
        dirty = true;
    }
    if cfg.ai.active_mmproj == filename {
        cfg.ai.active_mmproj.clear();
        dirty = true;
    }
    if dirty {
        settings::save(&pool, &cfg).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 返回 Hindsight 内置的推荐模型清单——前端拿来渲染推荐卡片。
/// 静态数据，不查 DB / 网络。
#[tauri::command]
pub async fn list_recommended_models() -> Result<Vec<Recommended>, String> {
    Ok(RECOMMENDED.to_vec())
}

/// 下载 GGUF 时进度事件 payload。前端 listen 这条事件名拿到。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadProgressPayload {
    /// 文件名，方便前端区分多个并发下载（main / mmproj）属于谁
    file: String,
    /// 已下载字节数
    downloaded: u64,
    /// 总字节数；HF 一般给 content-length
    total: Option<u64>,
}

const MODEL_PROGRESS_EVENT: &str = "ai://model-download-progress";

/// 从 HuggingFace 下载一个 GGUF 文件到当前 settings.ai.modelsPath。
///
/// - `repo` 形如 `ggml-org/Qwen2.5-VL-3B-Instruct-GGUF`
/// - `file` 是文件名（不含路径分隔符）
/// - `expected_bytes` 是预期字节数；用来判断"是否已下完整"的容差比对，
///    传 0 关闭这个检查
///
/// 进度通过 [`MODEL_PROGRESS_EVENT`] 流式推送给前端，命令本身在下载完才 resolve。
/// 返回值是落盘后的完整路径。
#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    pool: State<'_, DbPool>,
    repo: String,
    file: String,
    expected_bytes: u64,
) -> Result<String, String> {
    let cfg = settings::load(&pool).await.map_err(|e| e.to_string())?;
    let app_for_emit = app.clone();
    let file_for_emit = file.clone();
    let path = models::download_from_hf(
        &cfg.ai,
        &repo,
        &file,
        expected_bytes,
        move |downloaded, total| {
            let payload = ModelDownloadProgressPayload {
                file: file_for_emit.clone(),
                downloaded,
                total,
            };
            if let Err(e) = app_for_emit.emit(MODEL_PROGRESS_EVENT, &payload) {
                log::warn!("emit {MODEL_PROGRESS_EVENT} 失败: {e}");
            }
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// 进度事件 payload。前端 listen 这条事件名拿到。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgressPayload {
    phase: DownloadPhase,
    /// 已下载字节数；只在 phase=downloading 有意义，其它阶段是 0
    downloaded: u64,
    /// 总字节数；服务端不一定给，所以是 Option
    total: Option<u64>,
}

const PROGRESS_EVENT: &str = "ai://engine-download-progress";

/// 触发 llama-server binary 下载。
///
/// 进度通过 [`PROGRESS_EVENT`] 事件流式推送给前端，命令本身在下载结束后才 resolve。
/// 失败时返回错误字符串（前端 toast 展示）；同时事件流里**不会**再有 Done 信号，
/// 调用方应同时监听命令返回值和 Done 事件，按更早到的那个判定。
#[tauri::command]
pub async fn download_binary(app: AppHandle) -> Result<(), String> {
    let app_for_emit = app.clone();
    binary::download(move |phase, downloaded, total| {
        let payload = DownloadProgressPayload {
            phase,
            downloaded,
            total,
        };
        if let Err(e) = app_for_emit.emit(PROGRESS_EVENT, &payload) {
            // emit 失败不致命，后端仍继续干活；只是前端会丢这一帧进度
            log::warn!("emit {PROGRESS_EVENT} 失败: {e}");
        }
    })
    .await
    .map_err(Into::into)
}

/// 删除已安装的 binary（platform_dir 整个目录抹掉）。
#[tauri::command]
pub async fn delete_binary() -> Result<(), String> {
    binary::delete().map_err(Into::into)
}

/// 在系统文件管理器里打开 binary 所在目录。
///
/// 用 `open` crate（已在依赖里）调系统默认 file manager。
/// 未安装时也能开——目录可能存在（之前下载残留）但文件不全。
#[tauri::command]
pub async fn open_engine_dir() -> Result<(), String> {
    let bin = binary::binary_path().map_err(|e| e.to_string())?;
    let dir = bin
        .parent()
        .ok_or_else(|| "binary 路径无父目录".to_string())?;
    open::that(dir).map_err(|e| format!("打开目录失败：{e}"))?;
    Ok(())
}

// ─────────────────────────────────────────────────────────
//  AI 总结（Phase 1B-γ）
// ─────────────────────────────────────────────────────────

/// 跑某天的全部段总结。
///
/// 命令本体异步等到所有段完成才 resolve（或 cancel 后早 return）；
/// 期间通过 [`SUMMARY_PROGRESS_EVENT`] 流式推进度，前端 listen 边跑边渲染。
///
/// 重复触发：前端 UI 应防重复点；后端不加锁，但启动前会 reset cancel 标记，
/// 所以理论上后到的 generate 不会被前一次的 cancel 干掉。
#[tauri::command]
pub async fn generate_day_summary(
    app: AppHandle,
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    cancel: State<'_, SummaryCancel>,
    date: String,
    force_refresh: bool,
    device_id: Option<String>,
) -> Result<(), String> {
    let parsed_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let device = device_filter_from_option(device_id);

    cancel.0.store(false, Ordering::Relaxed);
    let runner = DaySummaryRunner::new(
        (*pool).clone(),
        Arc::clone(&supervisor),
        app.clone(),
        Arc::clone(&cancel.0),
    );

    if let Err(e) = runner.run(parsed_date, device, force_refresh).await {
        // 顶层失败也 emit 一条 error，前端 UI 能 toast
        let _ = app.emit(
            SUMMARY_PROGRESS_EVENT,
            &SummaryProgress {
                date: date.clone(),
                phase: "error",
                segment_idx: None,
                total_segments: 0,
                images_total: None,
                content: None,
                status: None,
                message: Some(e.to_string()),
            },
        );
        return Err(e.to_string());
    }
    Ok(())
}

/// 单段重试——只重跑指定一段，不动其它段。复用 supervisor 已经在跑的 server。
#[tauri::command]
pub async fn retry_summary_segment(
    app: AppHandle,
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    cancel: State<'_, SummaryCancel>,
    date: String,
    segment_idx: u32,
    device_id: Option<String>,
) -> Result<(), String> {
    let parsed_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("日期格式应为 YYYY-MM-DD：{e}"))?;
    let device = device_filter_from_option(device_id);

    cancel.0.store(false, Ordering::Relaxed);
    let runner = DaySummaryRunner::new(
        (*pool).clone(),
        Arc::clone(&supervisor),
        app,
        Arc::clone(&cancel.0),
    );
    runner
        .run_one_segment_only(parsed_date, segment_idx, device)
        .await
        .map_err(|e| e.to_string())
}

/// 设取消标记——下一段循环开头会感知到然后 Ok(()) 提早返回。
/// 已经在路上的单段 LLM 请求**不会**被中断（一段 30-180s 必须跑完）。
#[tauri::command]
pub async fn cancel_day_summary(cancel: State<'_, SummaryCancel>) -> Result<(), String> {
    cancel.0.store(true, Ordering::Relaxed);
    Ok(())
}

/// 拉某天已经落库的总结。前端进页面时调一次：有就直接渲染，没有就显示
/// "点击生成"按钮。
#[tauri::command]
pub async fn get_day_summary(
    pool: State<'_, DbPool>,
    date: String,
) -> Result<Vec<SegmentSummaryRow>, String> {
    ai_summaries::get_day(&pool, &date)
        .await
        .map_err(|e| e.to_string())
}
