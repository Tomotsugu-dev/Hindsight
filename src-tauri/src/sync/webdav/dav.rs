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

/// `PROPFIND`'s request body.
const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:"><D:prop><D:getlastmodified/><D:getcontentlength/><D:resourcetype/></D:prop></D:propfind>"#;

pub(crate) struct HttpDav {
    client: reqwest::Client,
    /// URL of the sync root, ending with `/`,
    /// e.g., `https://dav.jianguoyun.com/dav/hindsight/`.
    root: Url,
    username: String,
    password: String,
}

impl HttpDav {
    /// `base` is the server address provided by the user (e.g., `https://dav.jianguoyun.com/dav/`),
    /// must be HTTPS (ADR-0007). The root directory is fixed as [`ROOT_DIR`].
    pub(crate) fn new(base: &str, username: &str, password: &str) -> Result<Self> {
        let mut base = Url::parse(base.trim())
            .map_err(|e| Error::InvalidInputDyn(format!("WebDAV URL is invalid: {e}")))?;
        if base.scheme() != "https" {
            return Err(Error::InvalidInputDyn("WebDAV URL must be https".into()));
        }
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
            username: username.to_string(),
            password: password.to_string(),
        })
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
        let resp = req
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dav(base: &str) -> HttpDav {
        HttpDav::new(base, "alice", "secret").unwrap()
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
        assert!(HttpDav::new("http://dav.example.com/dav/", "a", "b").is_err());
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
