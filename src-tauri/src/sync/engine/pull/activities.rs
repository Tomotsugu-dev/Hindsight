use crate::capture::ignore::{is_excluded, IgnoreRule};
use crate::error::Result;
use crate::repo::categories::is_private;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::payload::ActivityPayload;

pub(super) async fn merge(
    pool: &DbPool,
    device_id: &str,
    local_date: &str,
    body: &[u8],
    ignore_rules: &[IgnoreRule],
) -> Result<()> {
    let s = std::str::from_utf8(body)?;
    // Rows present in this file; after the loop, the mirror rows not among them
    // are deleted.
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Skip that delete when a line failed to parse or `upsert_remote_activity`
    // failed on it: the local rows no longer match the file.
    let mut parse_clean = true;

    for (lineno, line) in s.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: ActivityPayload = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("activities line {lineno}: does not parse: {e}");
                parse_clean = false;
                continue;
            }
        };
        if is_private(pool, &row.process_name, row.url_host.as_deref()).await? {
            continue;
        }
        let remote_id = row.id.to_string();
        seen_ids.insert(remote_id.clone());
        let excluded = is_excluded(
            &row.process_name,
            row.window_title.as_deref().unwrap_or(""),
            ignore_rules,
        );
        if let Err(e) = upsert_remote_activity(pool, device_id, &row, excluded).await {
            log::warn!("activities line {lineno}: upsert failed: {e}");
            parse_clean = false;
        }
    }

    // Never delete rows the device still has; delete only the activity rows it
    // no longer has. Delete nothing when the file did not fully land.
    if !parse_clean {
        return Ok(());
    }
    let device_id_db = device_id.to_string();
    let local_date_db = local_date.to_string();
    let ids_vec: Vec<String> = seen_ids.into_iter().collect();
    let deleted = pool
        .0
        .call(move |conn| {
            // SQL cannot bind a list, so one placeholder per id.
            let placeholders: String = if ids_vec.is_empty() {
                String::new()
            } else {
                std::iter::repeat_n("?", ids_vec.len())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let sql = if placeholders.is_empty() {
                "DELETE FROM activities
                 WHERE device_id = ?1 AND local_date = ?2"
                    .to_string()
            } else {
                format!(
                    "DELETE FROM activities
                     WHERE device_id = ?1 AND local_date = ?2
                       AND remote_id NOT IN ({placeholders})"
                )
            };
            let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(2 + ids_vec.len());
            params.push(&device_id_db);
            params.push(&local_date_db);
            for id in &ids_vec {
                params.push(id);
            }
            let n = conn.execute(&sql, params.as_slice()).db()?;
            Ok(n)
        })
        .await?;
    if deleted > 0 {
        log::info!(
            "activities of device {device_id} on {local_date}: deleted {deleted} rows no longer in its file"
        );
    }
    Ok(())
}

/// Writes one activity row from a file into the local table: inserted when this
/// device has no such row, overwritten in full when the file's row is newer.
async fn upsert_remote_activity(
    pool: &DbPool,
    device_id: &str,
    row: &ActivityPayload,
    excluded: bool,
) -> Result<()> {
    let device_id = device_id.to_string();
    let remote_id = row.id.to_string();
    let row = row.clone();
    pool.0
        .call(move |conn| {
            let existing: Option<(i64, String)> = conn
                .query_row(
                    "SELECT id, updated_at FROM activities
                     WHERE device_id = ?1 AND remote_id = ?2",
                    rusqlite::params![device_id, remote_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            let local_hour = row.local_hour as u8;
            let window_title = row.window_title.as_deref().unwrap_or("");
            match existing {
                None => {
                    // A row this device has not seen: insert it under an id of our own.
                    conn.execute(
                        "INSERT INTO activities(
                           started_at, ended_at, duration_secs, local_date, local_hour,
                           process_name, window_title, category_id, screenshot_path,
                           device_id, remote_id, updated_at, origin, excluded, url_host
                         ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, 'remote', ?, ?)",
                        rusqlite::params![
                            row.started_at,
                            row.ended_at,
                            row.duration_secs,
                            row.local_date,
                            local_hour,
                            row.process_name,
                            window_title,
                            row.category_id,
                            device_id,
                            remote_id,
                            row.updated_at,
                            excluded,
                            row.url_host,
                        ],
                    )
                    .db()?;
                }
                Some((id, cur_updated)) => {
                    if row.updated_at > cur_updated {
                        conn.execute(
                            "UPDATE activities SET
                               started_at = ?, ended_at = ?, duration_secs = ?,
                               local_date = ?, local_hour = ?,
                               process_name = ?, window_title = ?, category_id = ?,
                               updated_at = ?, url_host = ?
                             WHERE id = ?",
                            rusqlite::params![
                                row.started_at,
                                row.ended_at,
                                row.duration_secs,
                                row.local_date,
                                local_hour,
                                row.process_name,
                                window_title,
                                row.category_id,
                                row.updated_at,
                                row.url_host,
                                id,
                            ],
                        )
                        .db()?;
                    }
                }
            }
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::categories::assign_app;
    use crate::repo::test_util::fresh_test_pool;

    const DAY: &str = "2026-05-15";
    const OTHER_DEVICE: &str = "device-a";

    /// 跨设备路径 (device_id != self_id)：
    /// - ndjson 中的 id 全部覆盖到 mirror（UPDATE 已存在 / INSERT 新行）
    /// - ndjson 中**不在**的 remote_id 通过 mirror 收敛 DELETE 掉
    #[tokio::test]
    async fn merge_activities_cross_device_converges_mirror() {
        let pool = fresh_test_pool().await;
        seed_mirror_rows(&pool, OTHER_DEVICE, &["1", "2", "3", "4", "5"]).await;

        let body = ndjson_for_ids(&[1, 2, 3, 6]);
        merge(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();

        let ids = remote_ids_for(&pool, OTHER_DEVICE).await;
        assert_eq!(
            ids,
            vec!["1".to_string(), "2".into(), "3".into(), "6".into()],
            "ndjson 包含 1/2/3/6，不在的 4/5 应被 mirror 收敛 DELETE"
        );
    }

    /// 解析失败的 ndjson：mirror 收敛**不**触发，避免半截文件误删一大堆。
    #[tokio::test]
    async fn merge_activities_parse_failure_skips_mirror_convergence() {
        let pool = fresh_test_pool().await;
        seed_mirror_rows(&pool, OTHER_DEVICE, &["1", "2", "3", "4", "5"]).await;

        // 第二行故意写坏 JSON
        let body = format!(
            "{}\nthis is not valid json {{\n{}\n",
            payload_line(1),
            payload_line(2),
        );
        merge(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();

        let ids = remote_ids_for(&pool, OTHER_DEVICE).await;
        assert_eq!(
            ids,
            vec![
                "1".to_string(),
                "2".into(),
                "3".into(),
                "4".into(),
                "5".into(),
            ],
            "解析失败时 mirror 收敛应跳过：原 5 行全部保留"
        );
    }

    /// 合并期打标：对端行 INSERT 进本机时按**本机**规则判 excluded；
    /// 后续更新走 LWW UPDATE（不触碰 excluded 列），标记必须存活。
    #[tokio::test]
    async fn merge_activities_tags_excluded_and_lww_update_keeps_it() {
        let pool = fresh_test_pool().await;
        let rules = vec![crate::capture::ignore::IgnoreRule {
            process_name: "Code".into(), // payload_line 固定 process_name = "Code"
            title_keyword: None,
        }];

        let body = ndjson_for_ids(&[1]);
        merge(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &rules)
            .await
            .unwrap();
        assert_eq!(
            excluded_of(&pool, OTHER_DEVICE, "1").await,
            1,
            "命中本机规则的对端行 INSERT 时就该打标"
        );

        // 同一行的更新（updated_at 更晚 → 走 LWW UPDATE 分支），这次不带规则——
        // 模拟"对端不知道本机规则"的真实情况，标记不该被冲掉
        let newer = ActivityPayload {
            id: 1,
            started_at: format!("{DAY}T10:01:00Z"),
            ended_at: format!("{DAY}T10:05:00Z"),
            duration_secs: 240,
            local_date: DAY.into(),
            local_hour: 10,
            process_name: "Code".into(),
            window_title: None,
            category_id: "other".into(),
            updated_at: format!("{DAY}T23:59:59Z"),
            url_host: None,
        };
        let body = serde_json::to_string(&newer).unwrap();
        merge(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();
        let (dur, excl) = row_state(&pool, OTHER_DEVICE, "1").await;
        assert_eq!(dur, 240, "LWW UPDATE 应生效（duration 更新）");
        assert_eq!(excl, 1, "LWW UPDATE 不触碰 excluded——本机打的标必须存活");
    }

    /// 隐私的应用和隐私网站的行不写进本地。不写的行不算文件里有，所以以前拉来的那几行也删掉。
    #[tokio::test]
    async fn pull_skips_private_activity_rows() {
        let pool = fresh_test_pool().await;
        let body = [
            payload_of(1, "Tor", None),
            payload_of(2, "Firefox", Some("gemini.google.com")),
            payload_of(3, "Firefox", Some("github.com")),
        ]
        .join("\n");
        merge(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();
        assert_eq!(remote_ids_for(&pool, OTHER_DEVICE).await, ["1", "2", "3"]);

        assign_app(&pool, "Tor", "private").await.unwrap();
        crate::repo::site_rules::set(&pool, "google.com", "private")
            .await
            .unwrap();
        merge(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();

        assert_eq!(remote_ids_for(&pool, OTHER_DEVICE).await, ["3"]);
    }

    /// 文件里的一行：哪个应用、停在哪个网站。
    fn payload_of(id: i64, process: &str, host: Option<&str>) -> String {
        let p = ActivityPayload {
            id,
            started_at: format!("{DAY}T10:0{id}:00Z"),
            ended_at: format!("{DAY}T10:0{id}:30Z"),
            duration_secs: 30,
            local_date: DAY.into(),
            local_hour: 10,
            process_name: process.into(),
            window_title: None,
            category_id: "other".into(),
            updated_at: format!("{DAY}T10:0{id}:30Z"),
            url_host: host.map(str::to_string),
        };
        serde_json::to_string(&p).unwrap()
    }

    async fn excluded_of(pool: &DbPool, device_id: &str, remote_id: &str) -> i64 {
        row_state(pool, device_id, remote_id).await.1
    }

    async fn host_of(pool: &DbPool, device_id: &str, remote_id: &str) -> Option<String> {
        let device_id = device_id.to_string();
        let remote_id = remote_id.to_string();
        pool.0
            .call(move |conn| {
                let v = conn
                    .query_row(
                        "SELECT url_host FROM activities WHERE device_id = ?1 AND remote_id = ?2",
                        rusqlite::params![device_id, remote_id],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .db()?;
                Ok(v)
            })
            .await
            .unwrap()
    }

    /// url_host 随 ndjson 往返：INSERT 带上、LWW UPDATE 跟着来源更新；
    /// None 时不序列化该字段；老版本写的行（没有 urlHost）解析不报错、落 None。
    #[tokio::test]
    async fn merge_activities_carries_url_host() {
        let pool = fresh_test_pool().await;
        let mut p = ActivityPayload {
            id: 7,
            started_at: format!("{DAY}T10:00:00Z"),
            ended_at: format!("{DAY}T10:00:30Z"),
            duration_secs: 30,
            local_date: DAY.into(),
            local_hour: 10,
            process_name: "Google Chrome".into(),
            window_title: Some("Hindsight - GitHub".into()),
            category_id: "other".into(),
            updated_at: format!("{DAY}T10:00:30Z"),
            url_host: Some("github.com".into()),
        };
        let body = serde_json::to_string(&p).unwrap();
        assert!(body.contains("\"urlHost\":\"github.com\""), "{body}");
        merge(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();
        assert_eq!(
            host_of(&pool, OTHER_DEVICE, "7").await,
            Some("github.com".into()),
            "INSERT 应带域名"
        );

        // 来源设备重推更新的同一行（updated_at 更新）：域名随之更新
        p.updated_at = format!("{DAY}T11:00:00Z");
        p.url_host = Some("docs.github.com".into());
        let body = serde_json::to_string(&p).unwrap();
        merge(&pool, OTHER_DEVICE, DAY, body.as_bytes(), &[])
            .await
            .unwrap();
        assert_eq!(
            host_of(&pool, OTHER_DEVICE, "7").await,
            Some("docs.github.com".into()),
            "LWW UPDATE 应更新域名"
        );

        // None 不序列化；老版本 ndjson 没有该字段也能解析
        p.url_host = None;
        assert!(!serde_json::to_string(&p).unwrap().contains("urlHost"));
        let legacy = format!(
            r#"{{"id":8,"startedAt":"{DAY}T12:00:00Z","endedAt":"{DAY}T12:00:30Z","durationSecs":30,"localDate":"{DAY}","localHour":12,"processName":"Google Chrome","windowTitle":null,"categoryId":"other","updatedAt":"{DAY}T12:00:30Z"}}"#
        );
        merge(&pool, OTHER_DEVICE, DAY, legacy.as_bytes(), &[])
            .await
            .unwrap();
        assert_eq!(host_of(&pool, OTHER_DEVICE, "8").await, None);
    }

    async fn row_state(pool: &DbPool, device_id: &str, remote_id: &str) -> (i64, i64) {
        let device_id = device_id.to_string();
        let remote_id = remote_id.to_string();
        pool.0
            .call(move |conn| {
                let v = conn
                    .query_row(
                        "SELECT duration_secs, excluded FROM activities
                         WHERE device_id = ?1 AND remote_id = ?2",
                        rusqlite::params![device_id, remote_id],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .unwrap();
                Ok(v)
            })
            .await
            .unwrap()
    }

    fn payload_line(id: i64) -> String {
        let p = ActivityPayload {
            id,
            started_at: format!("{DAY}T10:0{id}:00Z"),
            ended_at: format!("{DAY}T10:0{id}:30Z"),
            duration_secs: 30,
            local_date: DAY.into(),
            local_hour: 10,
            process_name: "Code".into(),
            window_title: None,
            category_id: "other".into(),
            updated_at: format!("{DAY}T10:0{id}:30Z"),
            url_host: None,
        };
        serde_json::to_string(&p).unwrap()
    }

    fn ndjson_for_ids(ids: &[i64]) -> String {
        ids.iter()
            .map(|id| payload_line(*id))
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn seed_mirror_rows(pool: &DbPool, device_id: &str, remote_ids: &[&str]) {
        let device_id = device_id.to_string();
        let remote_ids: Vec<String> = remote_ids.iter().map(|s| s.to_string()).collect();
        pool.0
            .call(move |conn| {
                for r in &remote_ids {
                    conn.execute(
                        "INSERT INTO activities(
                            started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id, remote_id,
                            updated_at, origin
                         ) VALUES(
                            '2026-05-15T10:00:00Z', '2026-05-15T10:00:30Z', 30, '2026-05-15', 10,
                            'Code', '', 'other', ?1, ?2, '2026-05-15T10:00:00Z', 'remote'
                         )",
                        rusqlite::params![device_id, r],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn remote_ids_for(pool: &DbPool, device_id: &str) -> Vec<String> {
        let device_id = device_id.to_string();
        pool.0
            .call(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT remote_id FROM activities
                         WHERE device_id = ?1 ORDER BY remote_id",
                    )
                    .db()?;
                let rows = stmt
                    .query_map(rusqlite::params![device_id], |r| r.get::<_, String>(0))
                    .db()?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.db()?);
                }
                Ok(out)
            })
            .await
            .unwrap()
    }
}
