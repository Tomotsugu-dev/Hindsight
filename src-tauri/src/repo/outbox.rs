use chrono::Utc;
use rusqlite::{params, Connection};

/// outbox 操作类型
#[derive(Debug, Clone, Copy)]
pub enum OutboxOp {
    Upsert,
    /// 软删 / 整体删除占位；当前业务路径只走 Upsert（带 deleted_at），保留以备未来云端硬删
    #[allow(dead_code)]
    Delete,
}

impl OutboxOp {
    /// 序列化成 outbox 表里 `op` 字段的字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxOp::Upsert => "upsert",
            OutboxOp::Delete => "delete",
        }
    }
}

/// outbox 实体类型 —— 对应 Drive 上的文件 kind
#[derive(Debug, Clone, Copy)]
pub enum OutboxEntity {
    Activity,
    Category,
    AppCategory,
    ProcessPath,
    Device,
    AppIcon,
    AppGroup,
    AppGroupMember,
}

impl OutboxEntity {
    /// 序列化成 outbox 表 `entity` 字段的字符串，对应 Drive 上的文件 kind。
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxEntity::Activity => "activity",
            OutboxEntity::Category => "category",
            OutboxEntity::AppCategory => "app_category",
            OutboxEntity::ProcessPath => "process_path",
            OutboxEntity::Device => "device",
            OutboxEntity::AppIcon => "app_icon",
            OutboxEntity::AppGroup => "app_group",
            OutboxEntity::AppGroupMember => "app_group_member",
        }
    }
}

/// 在已有事务里写一条 outbox 行。业务表写入与这条 outbox 写入必须共享同一个 conn / 同一个事务，
/// 才能保证"业务持久化即同步可达"。
///
/// `payload` 是 JSON 字符串：upsert 时是当前行的快照；delete 时通常只需要 entity_pk，payload 可以为 "{}"。
pub fn enqueue(
    conn: &Connection,
    op: OutboxOp,
    entity: OutboxEntity,
    entity_pk: &str,
    payload: &str,
) -> rusqlite::Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO sync_outbox (op, entity, entity_pk, payload, created_at, attempts, next_retry_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?5)",
        params![op.as_str(), entity.as_str(), entity_pk, payload, now],
    )?;
    Ok(())
}
