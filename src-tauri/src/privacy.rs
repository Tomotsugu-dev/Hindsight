//! Commands delete the data of the `private` category through here: sync is paused first, then
//! capture drops the session it is recording, and then [`crate::repo::purge::purge`] runs.

use crate::capture::CaptureService;
use crate::error::Result;
use crate::memory::MemoryDb;
use crate::repo::purge::{self, PurgeReport, PurgeTarget};
use crate::repo::settings;
use crate::storage::DbPool;
use crate::sync::engine::SyncEngine;

/// Prefix of an error when the setting is saved but deleting the private data failed. The
/// frontend then says the old data is not all deleted, and offers to try again.
pub const ERR_PREFIX_PURGE_FAILED: &str = "[PURGE_FAILED] ";

/// Deletes all data of the apps and websites now in the `private` category, and returns how
/// much was deleted.
///
/// Sync is paused first: the deletion writes outbox rows and deletes from tables that push
/// reads. Then capture drops the activity row it is recording, because that row may be one of
/// the rows to delete.
pub async fn purge_private(
    pool: &DbPool,
    mem: Option<&MemoryDb>,
    svc: &CaptureService,
    engine: &SyncEngine,
) -> Result<PurgeReport> {
    let cfg = settings::load(pool).await?;
    let root = std::path::PathBuf::from(&cfg.screenshot_path);
    let _paused = engine.pause_flushes().await;
    let target = PurgeTarget::Private;
    svc.run_with_session_cleared(|| purge::purge(pool, mem, &root, engine.self_id(), &target))
        .await
}

/// Deletes the data that became private, after a change to a category, an app group or a
/// website rule. When the deletion fails, the error starts with [`ERR_PREFIX_PURGE_FAILED`].
pub async fn purge_private_after_change(
    pool: &DbPool,
    mem: Option<&MemoryDb>,
    svc: &CaptureService,
    engine: &SyncEngine,
) -> std::result::Result<(), String> {
    match purge_private(pool, mem, svc, engine).await {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("{ERR_PREFIX_PURGE_FAILED}{e}")),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::repo::categories::{self, is_private, PRIVATE_CATEGORY_ID};
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use crate::storage::SqliteResultExt;
    use crate::sync::cloud::drive::{DriveClient, InMemoryDriveStore};
    use crate::sync::cloud::CloudBackend;

    /// 清理要用到的同步引擎和采集服务。云端是内存里的假云端，采集不启动。
    fn engine_and_capture(pool: &DbPool) -> (SyncEngine, CaptureService) {
        let drive = DriveClient::with_fake_drive(Arc::new(InMemoryDriveStore::new()), pool.clone());
        let engine = SyncEngine::with_backend(
            pool.clone(),
            None,
            CloudBackend::from(drive),
            TEST_SELF_ID.into(),
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        (engine, CaptureService::new(pool.clone(), None, 30, tx))
    }

    /// 插一行活动：哪个应用、哪个网站、哪张截图。
    async fn seed_row(pool: &DbPool, app: &str, host: Option<&str>, screenshot: Option<&str>) {
        let (app, host, screenshot) = (
            app.to_string(),
            host.map(str::to_string),
            screenshot.map(str::to_string),
        );
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, window_title,
                                            category_id, url_host, screenshot_path)
                     VALUES('2026-11-05T10:00:00Z', '2026-11-05T10:05:00Z', 300, '2026-11-05',
                            10, ?1, 'title', 'other', ?2, ?3)",
                    rusqlite::params![app, host, screenshot],
                )
                .db()
            })
            .await
            .unwrap();
    }

    async fn activity_count(pool: &DbPool) -> i64 {
        pool.0
            .call(|conn| {
                conn.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0))
                    .db()
            })
            .await
            .unwrap()
    }

    /// 设置存好以后删除失败：设置留着，错误以 [PURGE_FAILED] 开头，前端据此提示重试。
    #[tokio::test]
    async fn purge_failure_keeps_the_setting_and_says_purge_failed() {
        let pool = fresh_test_pool().await;
        let dir = std::env::temp_dir().join(format!("hs-purge-failed-{}", std::process::id()));
        // 截图换成同名文件夹：删不掉，报的也不是「文件不存在」
        std::fs::create_dir_all(dir.join("stuck.png")).unwrap();
        let mut cfg = settings::load(&pool).await.unwrap();
        cfg.screenshot_path = dir.to_string_lossy().into_owned();
        settings::save(&pool, &cfg).await.unwrap();
        seed_row(&pool, "Tor", None, Some("stuck.png")).await;
        let (engine, svc) = engine_and_capture(&pool);

        categories::assign_app(&pool, "Tor", PRIVATE_CATEGORY_ID)
            .await
            .unwrap();
        let err = purge_private_after_change(&pool, None, &svc, &engine)
            .await
            .unwrap_err();

        assert!(err.starts_with(ERR_PREFIX_PURGE_FAILED), "{err}");
        assert!(is_private(&pool, "Tor", None).await.unwrap(), "设置留着");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 删掉子域名自己的规则以后，它跟随上级域名。上级是隐私的，它以前记下的行也删掉。
    #[tokio::test]
    async fn remove_site_rule_purges_a_subdomain_that_now_follows_private() {
        let pool = fresh_test_pool().await;
        crate::repo::site_rules::set(&pool, "google.com", PRIVATE_CATEGORY_ID)
            .await
            .unwrap();
        crate::repo::site_rules::set(&pool, "mail.google.com", "talk")
            .await
            .unwrap();
        seed_row(&pool, "Firefox", Some("mail.google.com"), None).await;
        let (engine, svc) = engine_and_capture(&pool);

        crate::repo::site_rules::remove(&pool, "mail.google.com")
            .await
            .unwrap();
        purge_private_after_change(&pool, None, &svc, &engine)
            .await
            .unwrap();

        assert_eq!(activity_count(&pool).await, 0);
    }
}
