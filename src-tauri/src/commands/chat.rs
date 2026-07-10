//! Chat 问答与会话管理命令。
//!
//! 问答:前端一问,后端跑完整个 agent 循环再一次性返回;历史落 memory.sqlite,
//! LLM 的多轮上下文由后端从库里读(库是唯一真源,前端不再维护历史镜像)。
//!
//! 路由(设计定稿:云端 first-class):
//! - 设置里启用了云端 API(`external_enabled` + endpoint/model 非空)→ 云端原生 tools;
//! - 否则走本地 llama-server(grammar 约束解码),按 step 2 文本模型 lazy 启动;
//! - 两边都不可用 → 明确报错引导用户去配置。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, State};

use super::screen_memory::MemoryState;
use crate::ai::server::EngineSupervisor;
use crate::chat::engine::{self, ChatAnswer};
use crate::chat::llm::ChatLlm;
use crate::chat::store::{self, ConversationMeta, StoredMessage};
use crate::chat::tools::ToolCtx;
use crate::memory::MemoryDb;
use crate::repo::settings;
use crate::storage::DbPool;

/// LLM 每步能看到的历史消息条数(user/assistant 各算一条)。
const HISTORY_TURNS: usize = 6;

/// 一次问答落库完成(或失败/被停止)时的广播事件。前端不论组件死活都靠它
/// 得知"答案已就绪,去库里刷新"——一次性返回模式下,promise 的宿主(组件)
/// 跳页/关窗就没了,这个事件是唯一可靠的送达通道。
pub const CHAT_ANSWER_READY_EVENT: &str = "chat:answer-ready";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnswerReadyPayload {
    conversation_id: i64,
    ask_id: String,
    /// true = 答案已入库;false = 失败或被停止(会话里是"有问无答",可重问)
    ok: bool,
}

/// 生成中问答的内存注册表,key = conversation_id。三个用途:
/// ① 同会话并发拒——第二问明确报"忙",不再静默排进本地引擎队列装死;
/// ② 真取消——停止按钮凭 ask_id 找到 sender,触发 select 分支丢弃生成 future
///   (reqwest 连接随之断开,llama-server 对断连会中止该 slot 的解码);
/// ③ 跳页/关窗后重开会话,前端查"是否仍在生成"以恢复打字指示。
#[derive(Default)]
pub struct ChatInflight(pub Mutex<HashMap<i64, InflightEntry>>);

pub struct InflightEntry {
    ask_id: String,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
}

/// chat_ask 的 RAII 收尾:不论正常返回、报错还是被取消,都从注册表摘除并
/// 广播 answer-ready(ok 由调用方在成功落库后置 true),提前 return 不会漏。
struct InflightGuard<'a> {
    map: &'a ChatInflight,
    app: tauri::AppHandle,
    conv_id: i64,
    ask_id: String,
    ok: bool,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        self.map.0.lock().unwrap().remove(&self.conv_id);
        let payload = AnswerReadyPayload {
            conversation_id: self.conv_id,
            ask_id: std::mem::take(&mut self.ask_id),
            ok: self.ok,
        };
        if let Err(e) = self.app.emit(CHAT_ANSWER_READY_EVENT, payload) {
            log::warn!("广播 answer-ready 失败: {e}");
        }
    }
}

/// 问答返回:答案平铺 + 会话 id(首条消息隐式建会话时,前端靠它接管)。
/// `cancelled` = 用户点了停止:answer 是空壳,前端只需清 loading,不渲染气泡。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAskResult {
    pub conversation_id: i64,
    pub cancelled: bool,
    #[serde(flatten)]
    pub answer: ChatAnswer,
}

fn require(mem: &MemoryState) -> Result<&MemoryDb, String> {
    mem.0
        .as_ref()
        .ok_or_else(|| "屏幕记忆库不可用(启动时打开失败,详见日志)".to_string())
}

/// 一次问答。`conversation_id` 为 None = 首条消息,隐式建会话(标题=首问截断)。
/// `ask_id` 由前端生成,是本次问答的取消句柄(首问时前端还不知道会话 id)。
// Tauri 命令的参数 = 注入的 State 们 + IPC 实参,拆参数结构体不合命令惯例
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn chat_ask(
    app: tauri::AppHandle,
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    mem: State<'_, MemoryState>,
    inflight: State<'_, ChatInflight>,
    question: String,
    conversation_id: Option<i64>,
    locale: Option<String>,
    ask_id: Option<String>,
) -> Result<ChatAskResult, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("问题不能为空".to_string());
    }
    let db = require(&mem)?;
    // 回答语言:跟随提问语言优先,界面语言(前端 i18n 传入)兜底
    let lang = crate::chat::lang::ChatLang::from_tag(locale.as_deref());
    let ask_id = ask_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // 解析会话:显式 id 验存在;None 隐式建(标题 = 首问截断)
    let conv_id = match conversation_id {
        Some(id) => {
            if !store::conversation_exists(db, id)
                .await
                .map_err(String::from)?
            {
                return Err("会话不存在(可能已被删除)".to_string());
            }
            id
        }
        None => store::create_conversation(db, &store::truncate_title(&question))
            .await
            .map_err(String::from)?,
    };

    // 注册 in-flight:同会话已有生成中的问答 → 明确报忙(本地引擎单槽,
    // 放进去也只是静默排队装死)。注册成功后由 guard 负责摘除 + 广播。
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut map = inflight.0.lock().unwrap();
        if map.contains_key(&conv_id) {
            return Err(lang.err_conversation_busy().to_string());
        }
        map.insert(
            conv_id,
            InflightEntry {
                ask_id: ask_id.clone(),
                cancel: Some(cancel_tx),
            },
        );
    }
    let mut guard = InflightGuard {
        map: inflight.inner(),
        app: app.clone(),
        conv_id,
        ask_id,
        ok: false,
    };

    // 先读历史(此时本条 user 消息尚未入库,不会自包含),再落提问——
    // LLM 失败时提问也保留,重载后显示"有问无答",可再问
    let history = store::recent_history(db, conv_id, HISTORY_TURNS)
        .await
        .map_err(String::from)?;
    store::append_user(db, conv_id, &question)
        .await
        .map_err(String::from)?;

    let cfg = settings::load(&pool).await.map_err(String::from)?;
    let ai = &cfg.ai;

    // 路由走独立的 chat 槽位:chat_main 空 = 自动(云端配好走云端,否则同 step 2);
    // sentinel = 明确云端;文件名 = 明确该本地模型
    let (llm, _inflight) = if ai.chat_use_cloud() {
        (
            ChatLlm::cloud(&ai.endpoint, ai.model.clone(), ai.api_key.clone())
                .map_err(String::from)?,
            None,
        )
    } else {
        let main_name = ai.effective_chat_main();
        if main_name.trim().is_empty() {
            return Err(
                "Chat 需要一个语言模型:请在设置里启用云端 API,或在「模型」里选一个本地模型"
                    .to_string(),
            );
        }
        let models_dir = crate::ai::models::root_dir(ai);
        let main_path = models_dir.join(main_name);
        if !main_path.exists() {
            return Err(format!("选中的本地模型不存在:{main_name}"));
        }
        // 显式选了 chat 专用模型 → 纯文本推理不带 mmproj;
        // 跟随 step 2 时沿用其 mmproj 配套(模型可能是 vision 权重)
        let follows_summary = {
            let c = ai.chat_main.trim();
            c.is_empty() || c == crate::ai::config::SUMMARY_CLOUD_SENTINEL
        };
        let mmproj_name = if follows_summary {
            ai.effective_summary_mmproj()
        } else {
            ""
        };
        let mmproj_path = (!mmproj_name.trim().is_empty())
            .then(|| models_dir.join(mmproj_name))
            .filter(|p| p.exists());
        // 引擎可能正载着别的模型(如段总结刚跑完)——不一致时换模型重启
        let needs_restart = supervisor
            .loaded_main()
            .map(|p| p != main_path)
            .unwrap_or(false);
        let port = if needs_restart {
            supervisor
                .restart_with_overrides(
                    Some(main_path),
                    mmproj_path,
                    crate::ai::server::EngineStartOverrides::default(),
                )
                .await
                .map_err(String::from)?
        } else {
            supervisor
                .start(Some(main_path), mmproj_path)
                .await
                .map_err(String::from)?
        };
        // 循环期间持 inference guard,防 idle watcher 中途杀掉 server
        let guard = supervisor.acquire_inference();
        (
            ChatLlm::local(port, main_name.to_string()).map_err(String::from)?,
            Some(guard),
        )
    };

    let ctx = ToolCtx::open_readonly().await.map_err(String::from)?;
    let today = chrono::Local::now().date_naive();
    // 生成可被"停止"打断:cancel 分支丢弃生成 future——reqwest 连接随之断开,
    // llama-server 对断连会中止解码;云端同理。提问已落库,什么都不补写,
    // 会话呈"有问无答"可重问;guard 广播 ok=false 让各处视图清掉打字指示。
    let answer = tokio::select! {
        _ = &mut cancel_rx => {
            log::info!("chat_ask 被用户停止(会话 {conv_id})");
            return Ok(ChatAskResult {
                conversation_id: conv_id,
                cancelled: true,
                answer: ChatAnswer {
                    text: String::new(),
                    citations: Vec::new(),
                    steps: 0,
                    degraded: false,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                },
            });
        }
        r = engine::answer(&llm, &ctx, &question, &history, today, lang) => {
            r.map_err(String::from)?
        }
    };

    store::append_assistant(
        db,
        conv_id,
        &answer.text,
        &answer.citations,
        answer.degraded,
        (answer.prompt_tokens, answer.completion_tokens),
    )
    .await
    .map_err(String::from)?;
    guard.ok = true;
    Ok(ChatAskResult {
        conversation_id: conv_id,
        cancelled: false,
        answer,
    })
}

/// 会话是否正在生成回答;是则返回该次问答的 ask_id(停止按钮的取消句柄)。
/// 跳页/关窗后重开会话时,前端靠它恢复"生成中"状态。
#[tauri::command]
pub fn chat_inflight(inflight: State<'_, ChatInflight>, conversation_id: i64) -> Option<String> {
    inflight
        .0
        .lock()
        .unwrap()
        .get(&conversation_id)
        .map(|e| e.ask_id.clone())
}

/// 停止一次生成。找不到(已完成/已停止)返回 false,幂等。
#[tauri::command]
pub fn chat_cancel(inflight: State<'_, ChatInflight>, ask_id: String) -> bool {
    let mut map = inflight.0.lock().unwrap();
    for entry in map.values_mut() {
        if entry.ask_id == ask_id {
            if let Some(tx) = entry.cancel.take() {
                let _ = tx.send(());
            }
            return true;
        }
    }
    false
}

/// 会话列表(最近更新在前)。
#[tauri::command]
pub async fn chat_list_conversations(
    mem: State<'_, MemoryState>,
) -> Result<Vec<ConversationMeta>, String> {
    let db = require(&mem)?;
    store::list_conversations(db).await.map_err(String::from)
}

/// 某会话的全部消息(时间正序)。
#[tauri::command]
pub async fn chat_get_messages(
    mem: State<'_, MemoryState>,
    conversation_id: i64,
) -> Result<Vec<StoredMessage>, String> {
    let db = require(&mem)?;
    store::get_messages(db, conversation_id)
        .await
        .map_err(String::from)
}

/// 重命名会话(空标题拒绝)。
#[tauri::command]
pub async fn chat_rename_conversation(
    mem: State<'_, MemoryState>,
    conversation_id: i64,
    title: String,
) -> Result<(), String> {
    let db = require(&mem)?;
    store::rename_conversation(db, conversation_id, &title)
        .await
        .map_err(String::from)
}

/// 删除会话及其全部消息。
#[tauri::command]
pub async fn chat_delete_conversation(
    mem: State<'_, MemoryState>,
    conversation_id: i64,
) -> Result<(), String> {
    let db = require(&mem)?;
    store::delete_conversation(db, conversation_id)
        .await
        .map_err(String::from)
}
