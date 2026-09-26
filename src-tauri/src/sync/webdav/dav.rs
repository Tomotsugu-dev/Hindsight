//! The interface to a WebDAV server. Paths are relative to the sync root.

use std::borrow::Cow;
use std::future::Future;

use reqwest::header::CONTENT_TYPE;
use reqwest::Method;
use url::Url;

use super::layout::{parse_multistatus, DavEntry};
use crate::error::{Error, Result};

/// Name of the sync root directory on the server,
/// appended to the user-provided base URL.
pub(crate) const ROOT_DIR: &str = "hindsight/";

pub(crate) trait DavOps: Send + Sync {
    /// List the contents of the specified directory (`Depth: 1`).
    /// `""` refers to the root directory. The returned `href` includes the server prefix and percent-encoding; stripping the prefix is the client's responsibility.
    fn propfind(&self, dir: &str) -> impl Future<Output = Result<Vec<DavEntry>>> + Send;
    fn get(&self, path: &str) -> impl Future<Output = Result<Vec<u8>>> + Send;
    fn put(&self, path: &str, body: Vec<u8>) -> impl Future<Output = Result<()>> + Send;
    /// move (rename) a file within the same server.
    /// If the target exists, it will be overwritten.
    fn mv(&self, from: &str, to: &str) -> impl Future<Output = Result<()>> + Send;
    /// Create a directory. Returns 405 if it already exists,
    /// 409 if the parent directory does not exist.
    /// The caller should handle these status codes, just like a real server would.
    fn mkcol(&self, dir: &str) -> impl Future<Output = Result<()>> + Send;
    /// Delete a file or directory. Succeeds even if the target does not exist.
    fn delete(&self, path: &str) -> impl Future<Output = Result<()>> + Send;
}

// ─────────────── HttpDav ───────────────

/// Parses the server address the user entered, e.g., `https://dav.jianguoyun.com/dav/`.
/// It must be HTTPS (ADR-0007).
pub(crate) fn parse_server_url(server_url: &str) -> Result<Url> {
    let url = Url::parse(server_url.trim())
        .map_err(|e| Error::InvalidInputDyn(format!("WebDAV URL is invalid: {e}")))?;
    if url.scheme() != "https" {
        return Err(Error::InvalidInputDyn("WebDAV URL must be https".into()));
    }
    Ok(url)
}

/// `PROPFIND`'s request body.
const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:"><D:prop><D:getlastmodified/><D:getcontentlength/><D:resourcetype/></D:prop></D:propfind>"#;

pub(crate) struct HttpDav {
    client: reqwest::Client,
    /// URL of the sync root, ending with `/`,
    /// e.g., `https://dav.jianguoyun.com/dav/hindsight/`.
    root: Url,
    /// User name and password. `WebDavClient::ensure_credential` puts them here
    /// at the start of each round, read from `auth_state`; empty until then.
    credentials: std::sync::Mutex<(String, String)>,
}

impl HttpDav {
    /// `server_url` is the server address provided by the user (e.g., `https://dav.jianguoyun.com/dav/`),
    /// must be HTTPS (ADR-0007). The root directory is fixed as [`ROOT_DIR`].
    pub(crate) fn new(server_url: &str) -> Result<Self> {
        let mut base = parse_server_url(server_url)?;
        // `join` follows URL resolution rules: `…/dav` + `hindsight/` gives
        // `…/hindsight/`, because `dav` is taken as a file name and replaced;
        // only `…/dav/` + `hindsight/` gives `…/dav/hindsight/`. Users often
        // enter the address without the trailing slash.
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let root = base
            .join(ROOT_DIR)
            .map_err(|e| Error::InvalidInputDyn(format!("WebDAV URL is invalid: {e}")))?;
        Ok(Self {
            client: reqwest::Client::new(),
            root,
            credentials: Default::default(),
        })
    }

    pub(crate) fn set_credentials(&self, user: String, password: String) {
        *self.credentials.lock().unwrap() = (user, password);
    }

    #[cfg(test)]
    pub(crate) fn credentials(&self) -> (String, String) {
        self.credentials.lock().unwrap().clone()
    }

    /// The server's host name, such as `dav.jianguoyun.com`.
    pub(crate) fn host(&self) -> &str {
        self.root.host_str().unwrap_or("")
    }

    /// relative file path → absolute URL.
    fn file_path_to_url(&self, path: &str) -> Result<Url> {
        let path = path.trim_start_matches('/');
        let encoded: Vec<Cow<str>> = path.split('/').map(urlencoding::encode).collect();
        self.root
            .join(&encoded.join("/"))
            .map_err(|e| Error::InvalidInputDyn(format!("WebDAV path is invalid {path}：{e}")))
    }

    /// relative directory path → absolute URL ending with `/`.
    /// `""` is the root itself.
    fn dir_path_to_url(&self, dir: &str) -> Result<Url> {
        let dir = dir.trim_matches('/');
        if dir.is_empty() {
            Ok(self.root.clone())
        } else {
            self.file_path_to_url(&format!("{dir}/"))
        }
    }

    /// Sends the request with Basic auth. A non-2xx status becomes
    /// [`Error::WebDavHttp`]; 207 Multi-Status counts as 2xx.
    async fn send(
        &self,
        stage: &'static str,
        req: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response> {
        let (user, password) = self.credentials.lock().unwrap().clone();
        let resp = req.basic_auth(user, Some(password)).send().await?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        Err(Error::WebDavHttp {
            stage,
            status,
            body,
        })
    }
}

impl DavOps for HttpDav {
    async fn propfind(&self, dir: &str) -> Result<Vec<DavEntry>> {
        let req = self
            .client
            .request(
                Method::from_bytes(b"PROPFIND").expect("PROPFIND is a valid method"),
                self.dir_path_to_url(dir)?,
            )
            .header("Depth", "1")
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(PROPFIND_BODY);
        let resp = self.send("propfind", req).await?;
        let bytes = resp.bytes().await?;
        parse_multistatus(&bytes)
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let req = self.client.get(self.file_path_to_url(path)?);
        let resp = self.send("get", req).await?;
        Ok(resp.bytes().await?.to_vec())
    }

    async fn put(&self, path: &str, body: Vec<u8>) -> Result<()> {
        let req = self
            .client
            .put(self.file_path_to_url(path)?)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(body);
        self.send("put", req).await?;
        Ok(())
    }

    async fn mv(&self, from: &str, to: &str) -> Result<()> {
        let req = self
            .client
            .request(
                Method::from_bytes(b"MOVE").expect("MOVE is a valid method"),
                self.file_path_to_url(from)?,
            )
            .header("Destination", self.file_path_to_url(to)?.as_str())
            .header("Overwrite", "T");
        self.send("move", req).await?;
        Ok(())
    }

    async fn mkcol(&self, dir: &str) -> Result<()> {
        let req = self.client.request(
            Method::from_bytes(b"MKCOL").expect("MKCOL is a valid method"),
            self.dir_path_to_url(dir)?,
        );
        self.send("mkcol", req).await?;
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let req = self.client.delete(self.file_path_to_url(path)?);
        match self.send("delete", req).await {
            Ok(_) => Ok(()),
            Err(Error::WebDavHttp { status: 404, .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

// ─────────────── Dav：生产还是假的 ───────────────

/// 生产走 HTTP，测试走假服务器。用枚举不用泛型：`CloudBackend` 里要放一个具体类型，
/// 端到端测试要让两台设备共用同一个假服务器。
pub(crate) enum Dav {
    Http(HttpDav),
    #[cfg(test)]
    Fake(std::sync::Arc<super::fake::FakeDav>),
}

impl Dav {
    /// Sets the user name and password for this round. The fake server does not
    /// check them.
    pub(crate) fn set_credentials(&self, user: String, password: String) {
        match self {
            Dav::Http(d) => d.set_credentials(user, password),
            #[cfg(test)]
            Dav::Fake(_) => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn fake(&self) -> &super::fake::FakeDav {
        match self {
            Dav::Fake(f) => f,
            Dav::Http(_) => panic!("not a fake server"),
        }
    }
}

impl DavOps for Dav {
    async fn propfind(&self, dir: &str) -> Result<Vec<DavEntry>> {
        match self {
            Dav::Http(d) => d.propfind(dir).await,
            #[cfg(test)]
            Dav::Fake(d) => d.propfind(dir).await,
        }
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        match self {
            Dav::Http(d) => d.get(path).await,
            #[cfg(test)]
            Dav::Fake(d) => d.get(path).await,
        }
    }

    async fn put(&self, path: &str, body: Vec<u8>) -> Result<()> {
        match self {
            Dav::Http(d) => d.put(path, body).await,
            #[cfg(test)]
            Dav::Fake(d) => d.put(path, body).await,
        }
    }

    async fn mv(&self, from: &str, to: &str) -> Result<()> {
        match self {
            Dav::Http(d) => d.mv(from, to).await,
            #[cfg(test)]
            Dav::Fake(d) => d.mv(from, to).await,
        }
    }

    async fn mkcol(&self, dir: &str) -> Result<()> {
        match self {
            Dav::Http(d) => d.mkcol(dir).await,
            #[cfg(test)]
            Dav::Fake(d) => d.mkcol(dir).await,
        }
    }

    async fn delete(&self, path: &str) -> Result<()> {
        match self {
            Dav::Http(d) => d.delete(path).await,
            #[cfg(test)]
            Dav::Fake(d) => d.delete(path).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dav(base: &str) -> HttpDav {
        HttpDav::new(base).unwrap()
    }

    /// 根目录 = 用户地址 + `hindsight/`；用户地址没尾斜杠也一样。
    #[test]
    fn root_is_base_plus_hindsight() {
        assert_eq!(
            dav("https://dav.example.com/dav/").root.as_str(),
            "https://dav.example.com/dav/hindsight/"
        );
        assert_eq!(
            dav("https://dav.example.com/dav").root.as_str(),
            "https://dav.example.com/dav/hindsight/"
        );
    }

    /// 明文 HTTP 拒绝（ADR-0007）。
    #[test]
    fn plain_http_is_refused() {
        assert!(HttpDav::new("http://dav.example.com/dav/").is_err());
    }

    /// 相对路径逐段编码，目录 URL 带尾斜杠，空目录就是根。
    #[test]
    fn urls_are_built_under_root() {
        let d = dav("https://dav.example.com/dav/");
        assert_eq!(
            d.file_path_to_url("abc/activities/2026/2026-09-20.ndjson")
                .unwrap()
                .as_str(),
            "https://dav.example.com/dav/hindsight/abc/activities/2026/2026-09-20.ndjson"
        );
        assert_eq!(
            d.file_path_to_url("manifest.a b.json").unwrap().as_str(),
            "https://dav.example.com/dav/hindsight/manifest.a%20b.json"
        );
        assert_eq!(
            d.file_path_to_url("abc/activities/").unwrap().as_str(),
            "https://dav.example.com/dav/hindsight/abc/activities/"
        );
        assert_eq!(
            d.file_path_to_url("").unwrap().as_str(),
            "https://dav.example.com/dav/hindsight/"
        );
    }
}
