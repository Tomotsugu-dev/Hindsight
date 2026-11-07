use crate::error::Result;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::ActivityPayload;

pub(super) async fn build(pool: &DbPool, self_id: &str, day: &str) -> Result<Vec<u8>> {
    let self_id = self_id.to_string();
    let day = day.to_string();
    let rows: Vec<ActivityPayload> = pool
        .0
        .call(move |conn| {
            // By the `device_id` column only: every row of this device goes into the
            // day file, and no peer's row is pushed back.
            let mut stmt = conn
                .prepare(
                    "SELECT id, started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, updated_at, url_host
                     FROM activities
                     WHERE device_id = ?1 AND local_date = ?2
                     ORDER BY id",
                )
                .db()?;
            let rows = stmt
                .query_map(rusqlite::params![self_id, day], |r| {
                    Ok(ActivityPayload {
                        id: r.get(0)?,
                        started_at: r.get(1)?,
                        ended_at: r.get(2)?,
                        duration_secs: r.get(3)?,
                        local_date: r.get(4)?,
                        local_hour: r.get(5)?,
                        process_name: r.get(6)?,
                        window_title: r.get(7)?,
                        category_id: r.get(8)?,
                        updated_at: r.get(9)?,
                        url_host: r.get(10)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;

    let mut out = Vec::with_capacity(rows.len() * 200);
    for row in &rows {
        let s = serde_json::to_string(row)?;
        out.extend_from_slice(s.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use chrono::Local;

    /// 测 [`build`] 的 device + date 过滤：
    /// - 只导出 `device_id = self_id` 的行
    /// - 跨日期不混（昨天的 self 行不能出现在今天的 ndjson）
    /// - 跨设备不混（对端 mirror 行不能被当作"本机贡献"重推）
    ///
    /// 这条防"mac 重推 Win 镜像数据"的 bug 重现。
    #[tokio::test]
    async fn build_activities_day_filters_by_device_and_date() {
        let pool = fresh_test_pool().await;
        let today = Local::now().format("%Y-%m-%d").to_string();
        let yesterday = (Local::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();

        // 3 行 self + today，process_name 各不同
        for process in ["Code", "Chrome", "Slack"] {
            insert_sealed(&pool, TEST_SELF_ID, &today, process, "local").await;
        }
        // 2 行 other device + today，origin='remote'（mac pull 来的镜像 —— 不能被回推）
        for process in ["Win-only-A", "Win-only-B"] {
            insert_sealed(&pool, "device-other", &today, process, "remote").await;
        }
        // 1 行 self 但是昨天 —— 当天的 ndjson 不该含它
        insert_sealed(&pool, TEST_SELF_ID, &yesterday, "Yesterday-app", "local").await;

        let body = build(&pool, TEST_SELF_ID, &today).await.unwrap();

        let lines: Vec<&[u8]> = body
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(lines.len(), 3, "应仅导出 3 行 (self + today)");

        let processes: Vec<String> = lines
            .iter()
            .map(|l| {
                let p: ActivityPayload = serde_json::from_slice(l).unwrap();
                p.process_name
            })
            .collect();

        let expected: std::collections::HashSet<&str> =
            ["Code", "Chrome", "Slack"].into_iter().collect();
        let got: std::collections::HashSet<&str> = processes.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            got, expected,
            "导出的 process_name 集合应正好是 self+today 的 3 个"
        );

        // 显式断言对端 + 昨日的没漏出
        for forbidden in ["Win-only-A", "Win-only-B", "Yesterday-app"] {
            assert!(
                !processes.iter().any(|p| p == forbidden),
                "不应导出 process_name={forbidden}（要么跨设备要么跨日期）"
            );
        }
    }

    async fn insert_sealed(
        pool: &DbPool,
        device_id: &str,
        local_date: &str,
        process_name: &str,
        origin: &str,
    ) {
        let device_id = device_id.to_string();
        let local_date = local_date.to_string();
        let process_name = process_name.to_string();
        let origin = origin.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(
                        ?1 || 'T10:00:00Z', ?1 || 'T10:00:30Z', 30, ?1, 10,
                        ?2, '', 'other', ?3, ?1 || 'T10:00:30Z', ?4
                     )",
                    rusqlite::params![local_date, process_name, device_id, origin],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }
}
