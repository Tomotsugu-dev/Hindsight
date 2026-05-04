mod bootstrap;
mod capture;
mod commands;
mod db;
mod device;
mod error;
mod icons;
mod permissions;
mod platform;
mod repo;
mod storage;
mod sync;

use std::sync::Arc;

use capture::CaptureService;
use db::SqliteResultExt;
use repo::{activities, devices, settings};
use storage::{db_path, DbPool};
use sync::engine::SyncEngine;
use tauri::Manager;

const CLEANUP_INTERVAL_SECS: u64 = 24 * 60 * 60;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 默认开 hindsight=info（用户可用 RUST_LOG 覆盖）。隐私命中 / 同步错误 / 启动信息
    // 都是 info 级别，普通运行也能在终端看到。
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("hindsight=info"),
    )
    .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let handle = app.handle().clone();

            if let Some(main_window) = handle.get_webview_window("main") {
                platform::apply_window_tweaks(&main_window);
            }

            tauri::async_runtime::block_on(async move {
                // 0) 平台权限：macOS 上的 Screen Recording。没拿到 xcap 拿不到其它进程
                //    的窗口，焦点采集功能整个废掉，但不会报错（CG API 静默降级），所以
                //    必须先在启动早期触发系统弹框请求权限。
                //    阻塞调用：用户没决定前 macOS 不返回。授权后 TCC 在下次进程启动时缓存。
                let perm = permissions::ensure_screen_recording();
                log::info!("Screen Recording permission: {:?}", perm);

                // 1) 启动级身份：必须在打开 DB 之前确定 device_id（device.json 在 DB 之外）
                let dev_meta = device::ensure_loaded()
                    .expect("加载设备身份")
                    .clone();
                log::info!("device_id: {}", dev_meta.device_id);

                // 2) 数据根 + DB
                let path = db_path().expect("解析数据库路径");
                log::info!("数据库路径: {}", path.display());

                let pool = DbPool::open(&path).await.expect("打开数据库");
                storage::migrations::run(&pool)
                    .await
                    .expect("执行数据库迁移");

                // 3) 把当前机器写进 devices 表（is_self=1）+ outbox
                devices::upsert_self(
                    &pool,
                    dev_meta.device_id.clone(),
                    dev_meta.display_name.clone(),
                    dev_meta.color.clone(),
                    dev_meta.icon.clone(),
                    dev_meta.os.clone(),
                )
                .await
                .expect("注册当前设备");

                // 3b) v8 之前硬编码的 'local' device_id 改成真实 self id（幂等，对老数据一次性生效）
                let self_id_for_fix = dev_meta.device_id.clone();
                let _ = pool
                    .0
                    .call(move |conn| {
                        let n = conn
                            .execute(
                                "UPDATE activities SET device_id = ?1 WHERE device_id = 'local'",
                                rusqlite::params![self_id_for_fix],
                            )
                            .db()?;
                        if n > 0 {
                            log::info!("把 {} 条 v8 之前的历史活动 device_id 改为 self", n);
                        }
                        Ok(())
                    })
                    .await;

                let cfg = settings::load(&pool).await.expect("读取设置");

                let svc = Arc::new(CaptureService::new(
                    pool.clone(),
                    cfg.capture_interval_seconds,
                ));
                svc.set_work_hours(cfg.work_hours_enabled, cfg.work_ranges.clone())
                    .await;
                svc.set_screenshot_config(
                    cfg.capture_enabled,
                    cfg.screenshot_path.clone(),
                    1280,
                    720,
                    80,
                )
                .await;
                svc.set_privacy_keywords(
                    cfg.privacy_url_keywords.clone(),
                    cfg.privacy_app_keywords.clone(),
                )
                .await;
                if cfg.capture_enabled {
                    svc.start().await;
                }

                spawn_cleanup_task(pool.clone());

                // 一次性 backfill：把老用户已经在文件 cache 里、但 app_icons 表里没行的图标
                // 灌进 DB + 入 outbox。否则这些"开启同步前提取过的图标"对端永远拉不到。
                // 后台跑，不挡启动；已存在的会跳过，重复启动开销很低。
                let pool_for_backfill = pool.clone();
                tokio::spawn(async move {
                    match repo::app_icons::backfill_db_from_cache_or_extract(&pool_for_backfill).await {
                        Ok(n) if n > 0 => log::info!("icon backfill: 新增 {n} 行 app_icons"),
                        Ok(_) => {}
                        Err(e) => log::warn!("icon backfill 失败: {e}"),
                    }
                });

                // 4) 同步引擎：登录态由 engine 内部检查，未登录时所有循环都是 no-op；
                //    所以可以无条件 start，登录后自动开始推
                let sync_engine = Arc::new(SyncEngine::new(pool.clone()));
                sync_engine.start().await;

                handle.manage(pool);
                handle.manage(svc);
                handle.manage(sync_engine);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::capture::start_capture,
            commands::capture::stop_capture,
            commands::capture::get_capture_status,
            commands::data::get_day_hours,
            commands::data::get_day_apps,
            commands::data::get_week_days,
            commands::data::get_week_apps,
            commands::data::get_month_days,
            commands::data::get_month_apps,
            commands::categories::list_categories,
            commands::categories::create_category,
            commands::categories::update_category,
            commands::categories::delete_category,
            commands::categories::reorder_categories,
            commands::categories::assign_app_to_category,
            commands::categories::unassign_app,
            commands::categories::list_unclassified_apps,
            commands::app_groups::list_app_groups,
            commands::app_groups::create_app_group,
            commands::app_groups::delete_app_group,
            commands::app_groups::merge_app_group,
            commands::app_groups::unmerge_app_group,
            commands::app_groups::rename_app_group,
            commands::app_groups::assign_app_group_category,
            commands::icons::get_app_icon,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::storage::get_storage_info,
            commands::storage::purge_activities,
            commands::storage::purge_screenshots,
            commands::storage::open_screenshots_dir,
            commands::storage::get_data_root,
            commands::storage::set_data_root,
            commands::devices::list_devices,
            commands::devices::update_self_device,
            commands::auth::auth_status,
            commands::auth::sign_in_with_google,
            commands::auth::sign_out,
            commands::sync::sync_status,
            commands::sync::sync_now,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

fn spawn_cleanup_task(pool: DbPool) {
    tokio::spawn(async move {
        loop {
            match settings::load(&pool).await {
                Ok(cfg) => {
                    match activities::delete_screenshots_older_than(&pool, cfg.retention_days).await {
                        Ok(n) if n > 0 => log::info!("清理了 {n} 张过期截图"),
                        Ok(_) => {}
                        Err(e) => log::warn!("清理过期截图失败: {e}"),
                    }
                }
                Err(e) => log::warn!("清理任务读取设置失败: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(CLEANUP_INTERVAL_SECS)).await;
        }
    });
}
