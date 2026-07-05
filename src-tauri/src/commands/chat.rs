//! Chat 问答与会话管理命令。
//!
//! 问答:前端一问,后端跑完整个 agent 循环再一次性返回;历史落 memory.sqlite,
//! LLM 的多轮上下文由后端从库里读(库是唯一真源,前端不再维护历史镜像)。
//!
//! 路由(设计定稿:云端 first-class):
//! - 设置里启用了云端 API(`external_enabled` + endpoint/model 非空)→ 云端原生 tools;
//! - 否则走本地 llama-server(grammar 约束解码),按 step 2 文本模型 lazy 启动;
//! - 两边都不可用 → 明确报错引导用户去配置。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

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

/// 问答返回:答案平铺 + 会话 id(首条消息隐式建会话时,前端靠它接管)。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAskResult {
    pub conversation_id: i64,
    #[serde(flatten)]
    pub answer: ChatAnswer,
}

fn require(mem: &MemoryState) -> Result<&MemoryDb, String> {
    mem.0
        .as_ref()
        .ok_or_else(|| "屏幕记忆库不可用(启动时打开失败,详见日志)".to_string())
}

/// 一次问答。`conversation_id` 为 None = 首条消息,隐式建会话(标题=首问截断)。
#[tauri::command]
pub async fn chat_ask(
    pool: State<'_, DbPool>,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    mem: State<'_, MemoryState>,
    question: String,
    conversation_id: Option<i64>,
) -> Result<ChatAskResult, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("问题不能为空".to_string());
    }
    let db = require(&mem)?;

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
    let answer = engine::answer(&llm, &ctx, &question, &history, today)
        .await
        .map_err(String::from)?;

    store::append_assistant(
        db,
        conv_id,
        &answer.text,
        &answer.citations,
        answer.degraded,
    )
    .await
    .map_err(String::from)?;
    Ok(ChatAskResult {
        conversation_id: conv_id,
        answer,
    })
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
