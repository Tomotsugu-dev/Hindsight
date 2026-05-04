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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCategoryPayload {
    pub process_name: String,
    pub category_id: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessPathPayload {
    pub process_name: String,
    pub exe_path: String,
    pub seen_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIconPayload {
    pub process_name: String,
    /// PNG 字节用 base64 标准编码塞进 JSON
    pub icon_png_base64: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGroupPayload {
    pub id: String,
    pub display_name: String,
    pub category_id: Option<String>,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGroupMemberPayload {
    pub process_name: String,
    pub group_id: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

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
