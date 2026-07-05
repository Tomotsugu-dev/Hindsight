//! 跨设备同步 JSON 的**单源真相结构体**。
//!
//! 同一张表的字段名以前散落在三处：
//!   - repo/*.rs 里的 _payload() 帮手（构造 outbox 行）
//!   - sync/engine/push.rs build_* 里的 json!({...})（DB → Drive JSON）
//!   - sync/engine/pull.rs merge_* 里的 .get("xxx").as_str()（Drive JSON → DB）
//!
//! 每加一列就要三处人肉同步，曾经因此漏掉了 categories.sort_order，
//! 跨设备拖拽重排被静默丢弃。本模块把 schema 收口到一个 struct 里，
//! 漏字段编译期就报。
//!
//! 结构体不持有 DB 状态：纯 DTO，serde rename_all = camelCase 让 Rust 命名
//! 转回 JSON 同款。

use serde::{Deserialize, Serialize};

/// 同步用的 categories 行 JSON 结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryPayload {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    pub builtin: bool,
    /// v16 引入；老对端推上来的没这个字段，pull 侧用 #[serde(default)] 兜底。
    #[serde(default)]
    pub sort_order: i64,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

/// app_categories 行的 JSON 形式（process_name → category_id 的 derived view）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCategoryPayload {
    pub process_name: String,
    pub category_id: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

/// process_paths 行的 JSON 形式（process_name → exe 路径），跨设备同步本机 exe 位置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessPathPayload {
    pub process_name: String,
    pub exe_path: String,
    pub seen_at: String,
    pub updated_at: String,
}

/// app_icons 行的 JSON 形式（PNG 字节 base64 编码后塞进 JSON）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIconPayload {
    pub process_name: String,
    /// PNG 字节用 base64 标准编码塞进 JSON
    pub icon_png_base64: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

/// app_groups 行的 JSON 形式（组主体：display_name + category_id）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGroupPayload {
    pub id: String,
    pub display_name: String,
    pub category_id: Option<String>,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

/// app_group_members 行的 JSON 形式（process_name → group_id）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGroupMemberPayload {
    pub process_name: String,
    pub group_id: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

/// activities 行的 JSON 形式（一段焦点会话）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPayload {
    pub id: i64,
    pub started_at: String,
    pub ended_at: String,
    pub duration_secs: i64,
    pub local_date: String,
    pub local_hour: i64,
    pub process_name: String,
    pub window_title: Option<String>,
    pub category_id: String,
    pub updated_at: String,
}

/// devices 行的 JSON 形式（device.json 同步过来的设备元数据）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceMetaPayload {
    pub device_id: String,
    pub display_name: String,
    pub color: String,
    pub icon: String,
    pub os: Option<String>,
    pub last_seen_at: Option<String>,
    pub updated_at: String,
}

/// 设备 tombstone 文件 (`device.<self_id>.tombstone.json`) 的 JSON 形式。
///
/// 由 [`crate::commands::storage::purge_cloud_data`] 在删完本机 Drive 文件之后上传一个，
/// 内容只有一个 `clearedAt` 时间戳。对端 pull 时识别 → 对该设备 mirror 行执行
/// `DELETE WHERE device_id=<owner> AND updated_at < clearedAt` —— 修补
/// 「Drive 上文件没了，对端的本地 mirror 仍残留」的 sync 协议缺陷
/// （sync 引擎只有 INSERT/UPDATE + 软删，没有"文件级缺失 → 行级 DELETE"的反向传播）。
///
/// 幂等：tombstone 文件本身留在 Drive 永久存在；对端反复 pull 看到它执行 DELETE 命中 0
/// 行（因为已经删过了）即 no-op。源端后续 capture 的新行 `updated_at > clearedAt`
/// 不会被 DELETE 影响。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TombstonePayload {
    /// RFC3339 字符串，purge_cloud_data 完成那一刻的时间
    pub cleared_at: String,
}

// ───────── 可选上云数据集(默认关;见 sync/engine/datasets.rs)─────────

/// ai_summaries 行(日报/周报文本)的 JSON 形式。LWW 键 = (source, localDate, segmentIdx),
/// 时间戳 = generatedAt(新生成的报告覆盖旧的)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSummaryPayload {
    pub source: String,
    pub local_date: String,
    pub segment_idx: u32,
    pub label: String,
    pub start_hour: u8,
    pub end_hour: u8,
    pub content: String,
    pub model: String,
    pub status: String,
    pub error: Option<String>,
    pub generated_at: String,
}

/// chat_conversations 行。guid 全局唯一;deleted_at 非空 = 墓碑(删除传播)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConversationPayload {
    pub guid: String,
    pub title: String,
    pub created_ts: String,
    pub updated_ts: String,
    pub deleted_at: Option<String>,
}

/// chat_messages 行。消息不可变:合并时按 guid INSERT OR IGNORE。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessagePayload {
    pub guid: String,
    pub conv_guid: String,
    pub role: String,
    pub content: String,
    /// Vec<Citation> 的 JSON 字符串(与本地列同构);user 消息为 None
    pub citations: Option<String>,
    pub degraded: bool,
    pub created_ts: String,
}

/// `device.<id>.chat.json` 的整体形状:会话(含墓碑)+ 存活会话的全部消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatFilePayload {
    pub conversations: Vec<ChatConversationPayload>,
    pub messages: Vec<ChatMessagePayload>,
}

/// text_sessions 行(屏幕记忆全文;不含 session_lines——证据帧路径只在源设备有意义)。
/// LWW 键 = guid,时间戳 = endedTs(会话在源设备随折叠增长,取新)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySessionPayload {
    pub guid: String,
    pub local_date: String,
    pub started_ts: String,
    pub ended_ts: String,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub text: String,
}
