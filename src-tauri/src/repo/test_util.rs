//! 单元测试共享 helper：开一个 in-memory SQLite + 跑完所有 schema migrations，
//! 拿到一个可直接用 repo 函数读写的 [`DbPool`]。
//!
//! 进程内所有 test 共享一份 `device::SELF_META`（OnceLock），约定固定 id
//! `"test-self-device"`；fixture 行的 `device_id` 也填它，才匹配 [`device::self_id`] 过滤。

use crate::storage::{migrations, DbPool};

/// 进程内所有单元测试共用的"本机"device_id。第一个测试调 [`fresh_test_pool`] 时
/// 通过 [`crate::device::init_for_tests`] 把它写入 `SELF_META`。
pub const TEST_SELF_ID: &str = "test-self-device";

/// 开一个新鲜的 in-memory SQLite + 跑完所有 schema migrations + 初始化
/// `device::self_id() == TEST_SELF_ID`。每个测试调一次，互不影响（in-memory
/// DB 是每个连接独立的）。
pub async fn fresh_test_pool() -> DbPool {
    let _ = crate::device::init_for_tests(TEST_SELF_ID);
    let pool = DbPool::open_in_memory()
        .await
        .expect("open in-memory sqlite");
    migrations::run(&pool).await.expect("run migrations");
    pool
}

/// 串行化所有会改 `HINDSIGHT_DATA_DIR` 的测试（device.rs 的 env 覆盖组、
/// settings.rs 的损坏 JSON 备份组）。这个 env var 是进程级全局，且
/// `bootstrap::data_root()` / `device::device_file()` 都是每次现读——cargo test
/// 并行跑时读写方分属不同模块，各模块自设 Mutex 锁不住彼此，锁必须放在这里共享。
/// 前一个测试带锁 panic 时锁会中毒；临界区只保护 env var，中毒后继续跑不会
/// 放大问题，直接取回内层 guard 避免连环红。
pub fn lock_data_dir_env() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
