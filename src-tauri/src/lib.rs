mod capture;
mod commands;
mod error;
mod icons;
mod repo;
mod storage;

use std::sync::Arc;

use capture::CaptureService;
use storage::{db_path, DbPool};
use tauri::Manager;

const DEFAULT_CAPTURE_INTERVAL_SECS: u32 = 10;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let path = db_path().expect("解析数据库路径");
                log::info!("数据库路径: {}", path.display());

                let pool = DbPool::open(&path).await.expect("打开数据库");
                storage::migrations::run(&pool)
                    .await
                    .expect("执行数据库迁移");

                let svc = Arc::new(CaptureService::new(
                    pool.clone(),
                    DEFAULT_CAPTURE_INTERVAL_SECS,
                ));
                svc.start().await;

                handle.manage(pool);
                handle.manage(svc);
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
            commands::categories::assign_app_to_category,
            commands::categories::unassign_app,
            commands::categories::list_unclassified_apps,
            commands::icons::get_app_icon,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
