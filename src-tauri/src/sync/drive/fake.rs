// ─────────────── InMemoryDriveStore：测试用的 mock Drive ───────────────

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::sync::cloud::FileMeta;

#[cfg(test)]
#[derive(Debug, Clone)]
struct StoredFile {
    name: String,
    content: Vec<u8>,
    modified_time: String,
}

/// 进程内 HashMap 模拟 Drive appDataFolder。语义精确镜像 Drive REST：
/// - 文件命名空间是扁平的（`device.<uuid>.<kind>...`）
/// - `upsert_by_name` 推进内部时钟，modifiedTime 单调递增
/// - `delete` 404 视为 Ok，与 HTTP 实现一致
/// - `list_appdata_files` 按 modifiedTime 升序 + 支持 modified_after 过滤
#[cfg(test)]
pub struct InMemoryDriveStore {
    files: Mutex<HashMap<String, StoredFile>>,
    next_id: AtomicU64,
    clock: Mutex<i64>,
    /// 测试注入开关：>0 时接下来 N 次 `upsert_by_name` 直接返回 500（模拟 Drive
    /// 瞬时故障），每失败一次消耗一次配额，归零后自动恢复正常。默认 0 = 不注入，
    /// 生产路径（HTTP 分支）完全不经过这里，默认行为不变。
    fail_next_upserts: AtomicU64,
    /// 这个假云端认不认当前用户。真后端从自己的凭证表判断，假后端没有那张表，
    /// 所以由测试用 [`Self::sign_out`] 直接拨。
    signed_in: AtomicBool,
}

#[cfg(test)]
impl InMemoryDriveStore {
    pub fn new() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            clock: Mutex::new(0),
            fail_next_upserts: AtomicU64::new(0),
            signed_in: AtomicBool::new(true),
        }
    }

    /// 让这个假云端当作用户没登录。仅测试用。
    #[allow(dead_code)]
    pub fn sign_out(&self) {
        self.signed_in.store(false, Ordering::SeqCst);
    }

    pub(crate) fn signed_in(&self) -> bool {
        self.signed_in.load(Ordering::SeqCst)
    }

    /// 注入：让接下来 `n` 次 [`Self::upsert_by_name`] 失败（500 瞬时错误）。
    /// 仅测试用 —— 验证 push 失败后 outbox 重试不变量。
    #[allow(dead_code)]
    pub fn fail_next_upserts(&self, n: u64) {
        self.fail_next_upserts.store(n, Ordering::SeqCst);
    }

    async fn next_modified_time(&self) -> String {
        let mut c = self.clock.lock().await;
        *c += 1;
        // 单调递增的 RFC3339，方便跟真实 Drive 的字典序一致。
        // 秒字段固定为 0、只让小数位增长：之前 `{:02}` 填 `*c % 60` 会在第 60 次
        // 上传时秒位回绕（...00.000060 < ...59.000059），破坏单调性
        format!("2026-05-15T10:00:00.{:09}Z", *c)
    }

    pub async fn list_appdata_files(&self, modified_after: &str) -> Result<Vec<FileMeta>> {
        let files = self.files.lock().await;
        let mut out: Vec<FileMeta> = files
            .iter()
            .filter(|(_, f)| modified_after.is_empty() || f.modified_time.as_str() > modified_after)
            .map(|(id, f)| FileMeta {
                id: id.clone(),
                name: f.name.clone(),
                modified_time: f.modified_time.clone(),
                size: Some(f.content.len() as u64),
            })
            .collect();
        out.sort_by(|a, b| a.modified_time.cmp(&b.modified_time));
        Ok(out)
    }

    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let files = self.files.lock().await;
        match files.get(file_id) {
            Some(f) => Ok(f.content.clone()),
            None => Err(Error::DriveHttp {
                stage: "InMemory download",
                status: 404,
                body: format!("file_id {file_id} not found"),
            }),
        }
    }

    pub async fn upsert_by_name(&self, name: &str, content: &[u8]) -> Result<String> {
        // 失败注入：配额 > 0 时原子扣减一次并返回 500。checked_sub 在 0 时返回
        // None → fetch_update Err → 不注入，走正常路径。
        if self
            .fail_next_upserts
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
            .is_ok()
        {
            return Err(Error::DriveHttp {
                stage: "InMemory upsert (injected failure)",
                status: 500,
                body: "injected transient failure".into(),
            });
        }
        let mt = self.next_modified_time().await;
        let mut files = self.files.lock().await;
        // 找现有 name 对应的 id
        let existing_id = files
            .iter()
            .find(|(_, f)| f.name == name)
            .map(|(id, _)| id.clone());
        let id = existing_id
            .unwrap_or_else(|| format!("mock-id-{}", self.next_id.fetch_add(1, Ordering::SeqCst)));
        files.insert(
            id.clone(),
            StoredFile {
                name: name.to_string(),
                content: content.to_vec(),
                modified_time: mt,
            },
        );
        Ok(id)
    }

    pub async fn delete(&self, file_id: &str) -> Result<()> {
        let mut files = self.files.lock().await;
        files.remove(file_id);
        Ok(())
    }
}

#[cfg(test)]
impl Default for InMemoryDriveStore {
    fn default() -> Self {
        Self::new()
    }
}
