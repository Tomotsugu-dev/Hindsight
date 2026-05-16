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
