use rusqlite::{params, Connection};

use crate::storage::utc_now_rfc3339;

/// Value written to the outbox `op` column. Push never reads it; a deletion
/// travels as an upsert of a row carrying `deleted_at`.
#[derive(Debug, Clone, Copy)]
pub enum OutboxOp {
    Upsert,
}

impl OutboxOp {
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxOp::Upsert => "upsert",
        }
    }
}

/// Value written to the outbox `entity` column; it decides which cloud file the
/// next push rewrites.
#[derive(Debug, Clone, Copy)]
pub enum OutboxEntity {
    Activity,
    Category,
    Device,
    AppIcon,
    AppGroup,
    AppGroupMember,
}

impl OutboxEntity {
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxEntity::Activity => "activity",
            OutboxEntity::Category => "category",
            OutboxEntity::Device => "device",
            OutboxEntity::AppIcon => "app_icon",
            OutboxEntity::AppGroup => "app_group",
            OutboxEntity::AppGroupMember => "app_group_member",
        }
    }
}

/// Marks the cloud file behind this entity as dirty, so the next push rewrites it.
///
/// Must run in the same transaction as the business write. If the table write
/// lands and this row does not, the file is never marked dirty, and the change
/// only goes out once some later change to the same file marks it dirty again.
///
/// An activity's `payload` must carry `localDate`: push uses it to find which
/// day's file to rewrite, and drops the row without it. Push does not read the
/// `payload` of any other entity.
pub fn enqueue(
    conn: &Connection,
    op: OutboxOp,
    entity: OutboxEntity,
    entity_pk: &str,
    payload: &str,
) -> rusqlite::Result<()> {
    let now = utc_now_rfc3339();
    conn.execute(
        "INSERT INTO sync_outbox (op, entity, entity_pk, payload, created_at, attempts, next_retry_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?5)",
        params![op.as_str(), entity.as_str(), entity_pk, payload, now],
    )?;
    Ok(())
}
