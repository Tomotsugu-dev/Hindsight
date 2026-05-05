//! AI 子系统的 Tauri 命令薄壳。
//!
//! 业务逻辑（如真正的 LLM 调用）会在 `crate::ai` 模块里实现，
//! 这里只做参数校验 / 错误归类 / 返回对前端友好的形状。

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::ai::binary::{self, DownloadPhase, EngineBinaryStatus};
use crate::ai::server::{EngineRuntimeStatus, EngineSupervisor};

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
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
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
fn fmt_send_err(e: reqwest::Error) -> String {
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

/// 启动 llama-server 子进程。Phase 1B-α 不传 model/mmproj，所以会以
/// "缺模型" 失败收场，supervisor 把 stderr 回吐给用户。Phase 1B-β
/// 起会改成接 settings.ai.activeModel 真传值进来。
#[tauri::command]
pub async fn start_engine(
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<u16, String> {
    supervisor.start(None, None).await.map_err(Into::into)
}

/// 停掉子进程（如果在跑）。
#[tauri::command]
pub async fn stop_engine(
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<(), String> {
    supervisor.stop().await.map_err(Into::into)
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
