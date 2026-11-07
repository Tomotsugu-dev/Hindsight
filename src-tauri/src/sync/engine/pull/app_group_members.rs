use super::{is_remote_newer, merge_lww_simple};
use crate::error::Result;
use crate::storage::DbPool;
use crate::sync::payload::AppGroupMemberPayload;

pub(super) async fn merge(pool: &DbPool, body: &[u8]) -> Result<()> {
    merge_lww_simple(
        pool,
        "app_group_member",
        body,
        |row: &AppGroupMemberPayload| {
            (!row.process_name.is_empty() && !row.group_id.is_empty())
                .then(|| row.process_name.clone())
        },
        |conn, row: AppGroupMemberPayload| {
            if !is_remote_newer(
                conn,
                "SELECT updated_at FROM app_group_members WHERE process_name = ?1",
                rusqlite::params![row.process_name],
                &row.updated_at,
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?, ?, ?, ?)
                 ON CONFLICT(process_name) DO UPDATE SET
                   group_id   = excluded.group_id,
                   updated_at = excluded.updated_at,
                   deleted_at = excluded.deleted_at",
                rusqlite::params![
                    row.process_name,
                    row.group_id,
                    row.updated_at,
                    row.deleted_at
                ],
            )?;
            Ok(())
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{exec_sql, read_row, s, T_MID, T_NEW, T_OLD};
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    /// [`merge`]:LWW 矩阵(含墓碑换组)。
    #[tokio::test]
    async fn merge_app_group_members_lww_matrix() {
        let pool = fresh_test_pool().await;
        for g in ["g1", "g2"] {
            exec_sql(
                &pool,
                "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                 VALUES(?1, ?1, NULL, ?2, NULL)",
                vec![s(g), s(T_OLD)],
            )
            .await;
        }
        for (proc, ts) in [("M-newer", T_OLD), ("M-older", T_NEW), ("M-equal", T_MID)] {
            exec_sql(
                &pool,
                "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                 VALUES(?1, 'g1', ?2, NULL)",
                vec![s(proc), s(ts)],
            )
            .await;
        }
        let remote_row = |p: &str, tomb: bool| AppGroupMemberPayload {
            process_name: p.into(),
            group_id: "g2".into(),
            updated_at: T_MID.into(),
            deleted_at: tomb.then(|| T_MID.into()),
        };
        let body = serde_json::to_vec(&vec![
            remote_row("M-newer", true),
            remote_row("M-older", false),
            remote_row("M-equal", false),
            remote_row("M-missing", false),
        ])
        .unwrap();
        merge(&pool, &body).await.unwrap();

        let get = |p: &'static str| {
            read_row(
                &pool,
                "SELECT group_id, updated_at, deleted_at FROM app_group_members WHERE process_name = ?1",
                p,
                3,
            )
        };
        assert_eq!(
            get("M-newer").await.unwrap(),
            vec![s("g2"), s(T_MID), s(T_MID)],
            "远端较新应覆盖(换组 + 墓碑)"
        );
        assert_eq!(
            get("M-older").await.unwrap(),
            vec![s("g1"), s(T_NEW), None],
            "远端较旧应保留本地"
        );
        assert_eq!(
            get("M-equal").await.unwrap(),
            vec![s("g1"), s(T_MID), None],
            "同 updated_at 严格不覆盖"
        );
        assert_eq!(
            get("M-missing").await.unwrap(),
            vec![s("g2"), s(T_MID), None],
            "本地缺行应插入"
        );
    }
}
