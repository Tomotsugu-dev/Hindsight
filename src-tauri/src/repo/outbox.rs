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
///
/// A new variant also goes into [`Self::ALL`]: [`Self::parse`] only knows the
/// variants listed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxEntity {
    Activity,
    Category,
    SiteRule,
    Device,
    AppIcon,
    AppGroup,
    AppGroupMember,
    SuperCategory,
    /// One day's screen-memory file of this device. Push drops these rows while
    /// screen-memory sync is off.
    Memory,
}

impl OutboxEntity {
    pub const ALL: [OutboxEntity; 9] = [
        OutboxEntity::Activity,
        OutboxEntity::Category,
        OutboxEntity::SiteRule,
        OutboxEntity::Device,
        OutboxEntity::AppIcon,
        OutboxEntity::AppGroup,
        OutboxEntity::AppGroupMember,
        OutboxEntity::SuperCategory,
        OutboxEntity::Memory,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            OutboxEntity::Activity => "activity",
            OutboxEntity::Category => "category",
            OutboxEntity::SiteRule => "site_rule",
            OutboxEntity::Device => "device",
            OutboxEntity::AppIcon => "app_icon",
            OutboxEntity::AppGroup => "app_group",
            OutboxEntity::AppGroupMember => "app_group_member",
            OutboxEntity::SuperCategory => "super_category",
            OutboxEntity::Memory => "memory",
        }
    }

    /// Returns the variant whose [`Self::as_str`] is `s`, or `None` when there
    /// is none.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == s)
    }
}

/// Marks the cloud file behind this entity as dirty, so the next push rewrites it.
///
/// Must run in the same transaction as the business write. If the table write
/// lands and this row does not, the file is never marked dirty, and the change
/// only goes out once some later change to the same file marks it dirty again.
///
/// An activity or memory row's `payload` must carry `localDate`: push uses it to
/// find which day's file to rewrite, and drops the row without it. Push does not
/// read the `payload` of any other entity.
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

/// Writes one `entity` row for each day, so the next push uploads this device's file for that
/// day again. `entity` must be one of the two kinds kept by day: activity or screen memory.
pub fn enqueue_days(
    conn: &Connection,
    entity: OutboxEntity,
    days: &[String],
) -> rusqlite::Result<()> {
    for day in days {
        let payload = serde_json::json!({ "localDate": day }).to_string();
        enqueue(conn, OutboxOp::Upsert, entity, day, &payload)?;
    }
    Ok(())
}

/// Used on a backend switch: makes the next push upload all of this device's
/// data to the new backend (ADR-0011 §2). One row for each day with activity on
/// this device, and one for each whole-table file.
///
/// Days are picked the same way push builds day files: by the `device_id`
/// column only.
pub fn enqueue_every_file(conn: &Connection, self_id: &str) -> rusqlite::Result<()> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT local_date FROM activities WHERE device_id = ?1")?;
    let days = stmt
        .query_map([self_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    enqueue_days(conn, OutboxEntity::Activity, &days)?;
    for entity in [
        OutboxEntity::Category,
        OutboxEntity::SiteRule,
        OutboxEntity::Device,
        OutboxEntity::AppIcon,
        OutboxEntity::AppGroup,
        OutboxEntity::AppGroupMember,
        OutboxEntity::SuperCategory,
    ] {
        enqueue(conn, OutboxOp::Upsert, entity, "*", "{}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个变体写进 outbox 的文本都能读回同一个变体。读不回的行，push 会当成认不出的丢掉。
    #[test]
    fn parse_reads_back_every_entity() {
        for entity in OutboxEntity::ALL {
            assert_eq!(OutboxEntity::parse(entity.as_str()), Some(entity));
        }
        assert_eq!(OutboxEntity::parse("no_such_entity"), None);
    }
}
