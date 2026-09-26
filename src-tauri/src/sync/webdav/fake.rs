//! 内存里的假 WebDAV 服务器，只在测试里用。状态码照真服务器：`put` 到父目录不存在
//! 的路径是 409，`mkcol` 已存在是 405，`get` 不存在是 404，`delete` 不存在算成功。
//! 每次写入推进一个秒级时钟，测试要制造「同一秒」时可以把时钟按住。
//! 每次调用都记下来，测试据此断言「发了哪些请求、按什么顺序」。
//! [`FakeDav::without_root`] 是新开的账号：同步根目录还不存在。
//! [`FakeDav::refuse_move_overwrite`] 让它像坚果云那样不许 `MOVE` 覆盖已有的文件。

#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use tokio::sync::Mutex;

use super::dav::DavOps;
use super::layout::DavEntry;
use crate::error::{Error, Result};

/// 一次调用，参数是相对路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Propfind(String),
    Get(String),
    Put(String),
    Move(String, String),
    Mkcol(String),
    Delete(String),
}

struct Stored {
    body: Vec<u8>,
    /// RFC3339，秒级，像真服务器那样。
    modified: String,
}

struct State {
    /// 相对路径 → 文件。
    files: BTreeMap<String, Stored>,
    /// 相对目录路径，不带尾斜杠；`""` 是根目录，一直存在。
    dirs: BTreeSet<String>,
    /// 服务器时钟：从固定起点数过去的秒数。
    clock: u64,
    hold_clock: bool,
    /// 像坚果云那样，`MOVE` 到已存在的文件回 409。
    refuse_move_overwrite: bool,
    /// 设了就让每个 `PUT` 回这个状态码和响应体，比如空间满的 507。
    fail_puts: Option<(u16, String)>,
    /// 设了就让这个目录下的 `GET` 回这个状态码；目录外的（比如根目录的 manifest 文件）照常。
    fail_gets_under: Option<(String, u16)>,
    calls: Vec<Call>,
}

pub(crate) struct FakeDav {
    /// 根目录在服务器上的路径，拼进 `href`，如 `/dav/hindsight/`。
    prefix: String,
    state: Mutex<State>,
}

impl FakeDav {
    pub(crate) fn new() -> Self {
        Self::with_prefix("/dav/hindsight/")
    }

    pub(crate) fn with_prefix(prefix: &str) -> Self {
        let mut dirs = BTreeSet::new();
        dirs.insert(String::new());
        Self {
            prefix: prefix.to_string(),
            state: Mutex::new(State {
                files: BTreeMap::new(),
                dirs,
                clock: 0,
                hold_clock: false,
                refuse_move_overwrite: false,
                fail_puts: None,
                fail_gets_under: None,
                calls: Vec::new(),
            }),
        }
    }

    /// 新开的账号：同步根目录还不存在，要有人 `MKCOL` 它。
    pub(crate) fn without_root() -> Self {
        let dav = Self::new();
        dav.state
            .try_lock()
            .expect("nobody else holds it yet")
            .dirs
            .clear();
        dav
    }

    /// 直接拿走一个文件，不记调用：模拟「服务器上这个文件暂时读不到」。
    pub(crate) async fn take_file(&self, path: &str) -> Option<Vec<u8>> {
        self.state.lock().await.files.remove(path).map(|f| f.body)
    }

    /// 直接放一个文件进去，缺的父目录一并建好；不记调用、时钟照常推进。
    pub(crate) async fn seed_file(&self, path: &str, body: &[u8]) -> String {
        let mut st = self.state.lock().await;
        let mut dir = String::new();
        for seg in parent_of(path).split('/').filter(|s| !s.is_empty()) {
            dir = if dir.is_empty() {
                seg.to_string()
            } else {
                format!("{dir}/{seg}")
            };
            st.dirs.insert(dir.clone());
        }
        let modified = st.tick();
        st.files.insert(
            path.to_string(),
            Stored {
                body: body.to_vec(),
                modified: modified.clone(),
            },
        );
        modified
    }

    /// 按住时钟：之后的写入都落在同一秒，直到放开。
    pub(crate) async fn hold_clock(&self, hold: bool) {
        self.state.lock().await.hold_clock = hold;
    }

    /// 像坚果云那样不许 `MOVE` 覆盖已有的文件。
    pub(crate) async fn refuse_move_overwrite(&self, refuse: bool) {
        self.state.lock().await.refuse_move_overwrite = refuse;
    }

    /// `Some((状态码, 响应体))` 让之后每个 `PUT` 都这样失败，`None` 恢复正常。
    pub(crate) async fn fail_puts(&self, failure: Option<(u16, &str)>) {
        self.state.lock().await.fail_puts =
            failure.map(|(status, body)| (status, body.to_string()));
    }

    /// `Some((目录, 状态码))` 让之后这个目录下的 `GET` 都这样失败，`None` 恢复正常。
    pub(crate) async fn fail_gets_under(&self, failure: Option<(&str, u16)>) {
        self.state.lock().await.fail_gets_under =
            failure.map(|(dir, status)| (dir.to_string(), status));
    }

    pub(crate) async fn calls(&self) -> Vec<Call> {
        self.state.lock().await.calls.clone()
    }

    pub(crate) async fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.state
            .lock()
            .await
            .files
            .get(path)
            .map(|f| f.body.clone())
    }

    fn href(&self, path: &str) -> String {
        format!("{}{}", self.prefix, path)
    }

    /// 目录的 href 以 `/` 结尾；根目录就是前缀本身。
    fn dir_href(&self, dir: &str) -> String {
        if dir.is_empty() {
            self.prefix.clone()
        } else {
            self.href(&format!("{dir}/"))
        }
    }
}

impl State {
    /// 推进一秒（按住时不推），返回当前时间。
    fn tick(&mut self) -> String {
        if !self.hold_clock {
            self.clock += 1;
        }
        let base = chrono::DateTime::parse_from_rfc3339("2026-05-15T10:00:00Z").unwrap();
        (base + chrono::Duration::seconds(self.clock as i64))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

fn http_err(stage: &'static str, status: u16, what: &str) -> Error {
    Error::WebDavHttp {
        stage,
        status,
        body: what.to_string(),
    }
}

impl DavOps for FakeDav {
    async fn propfind(&self, dir: &str) -> Result<Vec<DavEntry>> {
        let dir = dir.trim_matches('/');
        let mut st = self.state.lock().await;
        st.calls.push(Call::Propfind(dir.to_string()));
        if !st.dirs.contains(dir) {
            return Err(http_err("propfind", 404, dir));
        }
        let mut out = vec![DavEntry {
            href: self.dir_href(dir),
            is_dir: true,
            modified: None,
            size: None,
        }];
        for d in &st.dirs {
            if !d.is_empty() && parent_of(d) == dir {
                out.push(DavEntry {
                    href: self.dir_href(d),
                    is_dir: true,
                    modified: None,
                    size: None,
                });
            }
        }
        for (p, f) in &st.files {
            if parent_of(p) == dir {
                out.push(DavEntry {
                    href: self.href(p),
                    is_dir: false,
                    modified: Some(f.modified.clone()),
                    size: Some(f.body.len() as u64),
                });
            }
        }
        Ok(out)
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let mut st = self.state.lock().await;
        st.calls.push(Call::Get(path.to_string()));
        if let Some((dir, status)) = &st.fail_gets_under {
            if path.starts_with(dir.as_str()) {
                return Err(http_err("get", *status, path));
            }
        }
        st.files
            .get(path)
            .map(|f| f.body.clone())
            .ok_or_else(|| http_err("get", 404, path))
    }

    async fn put(&self, path: &str, body: Vec<u8>) -> Result<()> {
        let mut st = self.state.lock().await;
        st.calls.push(Call::Put(path.to_string()));
        if let Some((status, body)) = &st.fail_puts {
            return Err(http_err("put", *status, body));
        }
        if !st.dirs.contains(parent_of(path)) {
            return Err(http_err("put", 409, path));
        }
        let modified = st.tick();
        st.files.insert(path.to_string(), Stored { body, modified });
        Ok(())
    }

    async fn mv(&self, from: &str, to: &str) -> Result<()> {
        let mut st = self.state.lock().await;
        st.calls.push(Call::Move(from.to_string(), to.to_string()));
        if !st.dirs.contains(parent_of(to)) {
            return Err(http_err("move", 409, to));
        }
        if st.refuse_move_overwrite && st.files.contains_key(to) {
            return Err(http_err("move", 409, to));
        }
        let Some(mut f) = st.files.remove(from) else {
            return Err(http_err("move", 404, from));
        };
        f.modified = st.tick();
        st.files.insert(to.to_string(), f);
        Ok(())
    }

    async fn mkcol(&self, dir: &str) -> Result<()> {
        let dir = dir.trim_matches('/');
        let mut st = self.state.lock().await;
        st.calls.push(Call::Mkcol(dir.to_string()));
        if st.dirs.contains(dir) || st.files.contains_key(dir) {
            return Err(http_err("mkcol", 405, dir));
        }
        // 根目录的上一级是用户填的地址，一直在
        if !dir.is_empty() && !st.dirs.contains(parent_of(dir)) {
            return Err(http_err("mkcol", 409, dir));
        }
        st.dirs.insert(dir.to_string());
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let path = path.trim_matches('/');
        let mut st = self.state.lock().await;
        st.calls.push(Call::Delete(path.to_string()));
        if st.files.remove(path).is_some() {
            return Ok(());
        }
        // 目录：连同下面的一起删；根目录不删
        if !path.is_empty() && st.dirs.remove(path) {
            let under = format!("{path}/");
            st.dirs.retain(|d| !d.starts_with(&under));
            st.files.retain(|p, _| !p.starts_with(&under));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 父目录不存在 PUT 是 409；建好目录再 PUT 就成功，时钟推进一秒。
    #[tokio::test]
    async fn put_needs_the_parent_directory() {
        let dav = FakeDav::new();
        assert!(matches!(
            dav.put("abc/categories.json", b"[]".to_vec()).await,
            Err(Error::WebDavHttp { status: 409, .. })
        ));
        dav.mkcol("abc").await.unwrap();
        dav.put("abc/categories.json", b"[]".to_vec())
            .await
            .unwrap();
        let listed = dav.propfind("abc").await.unwrap();
        assert_eq!(listed[1].href, "/dav/hindsight/abc/categories.json");
        assert_eq!(listed[1].modified.as_deref(), Some("2026-05-15T10:00:01Z"));
    }

    /// 已存在的目录 MKCOL 是 405，父目录不存在是 409。
    #[tokio::test]
    async fn mkcol_status_codes_follow_the_server() {
        let dav = FakeDav::new();
        dav.mkcol("abc").await.unwrap();
        assert!(matches!(
            dav.mkcol("abc").await,
            Err(Error::WebDavHttp { status: 405, .. })
        ));
        assert!(matches!(
            dav.mkcol("x/y").await,
            Err(Error::WebDavHttp { status: 409, .. })
        ));
    }

    /// 新账号：列根目录是 404，往里 PUT 是 409；`MKCOL` 根目录之后才能用。
    #[tokio::test]
    async fn a_new_account_has_no_root_until_mkcol() {
        let dav = FakeDav::without_root();
        assert!(matches!(
            dav.propfind("").await,
            Err(Error::WebDavHttp { status: 404, .. })
        ));
        assert!(matches!(
            dav.put("a.json", b"{}".to_vec()).await,
            Err(Error::WebDavHttp { status: 409, .. })
        ));
        dav.mkcol("").await.unwrap();
        dav.put("a.json", b"{}".to_vec()).await.unwrap();
        assert_eq!(dav.propfind("").await.unwrap().len(), 2);
    }

    /// 列一层：第一条是目录自己，然后是子目录和文件，都带前缀；不下钻。
    #[tokio::test]
    async fn propfind_lists_one_level_with_prefix() {
        let dav = FakeDav::new();
        dav.seed_file("manifest.abc.json", b"{}").await;
        dav.seed_file("abc/activities/2026/2026-09-20.ndjson", b"x")
            .await;
        let hrefs: Vec<String> = dav
            .propfind("")
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.href)
            .collect();
        assert_eq!(
            hrefs,
            vec![
                "/dav/hindsight/",
                "/dav/hindsight/abc/",
                "/dav/hindsight/manifest.abc.json"
            ]
        );
    }

    /// MOVE 改名、覆盖；删不存在的东西算成功；按住时钟后两次写入同一秒。
    #[tokio::test]
    async fn move_delete_and_held_clock() {
        let dav = FakeDav::new();
        dav.mkcol("abc").await.unwrap();
        dav.put("abc/.tmp-a.json", b"new".to_vec()).await.unwrap();
        dav.put("abc/a.json", b"old".to_vec()).await.unwrap();
        dav.mv("abc/.tmp-a.json", "abc/a.json").await.unwrap();
        assert_eq!(dav.file("abc/a.json").await.as_deref(), Some(&b"new"[..]));
        assert_eq!(dav.file("abc/.tmp-a.json").await, None);
        dav.delete("abc/nothing.json").await.unwrap();

        dav.hold_clock(true).await;
        let t1 = dav.seed_file("abc/b.json", b"1").await;
        let t2 = dav.seed_file("abc/c.json", b"2").await;
        assert_eq!(t1, t2);

        assert_eq!(
            dav.calls().await,
            vec![
                Call::Mkcol("abc".into()),
                Call::Put("abc/.tmp-a.json".into()),
                Call::Put("abc/a.json".into()),
                Call::Move("abc/.tmp-a.json".into(), "abc/a.json".into()),
                Call::Delete("abc/nothing.json".into()),
            ]
        );
    }
}
