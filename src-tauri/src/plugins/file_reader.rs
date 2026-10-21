//! Plugin file reader: when the WebView asks for a plugin file with
//! `hindsight-plugin://<plugin id>.localhost/<version>/<file>`, it reads the file from the plugin
//! library and hands it over.
//!
//! On Windows the WebView uses `http://hindsight-plugin.<plugin id>.localhost/…`. Tauri turns it
//! back into the form above before the request reaches this module.

use std::fs;
use std::path::{Component, Path, PathBuf};

use tauri::http::{header, Response, StatusCode, Uri};
use tauri::utils::mime_type::MimeType;

use crate::error::{Error, Result};
use crate::plugins::manifest::{check_id, check_version};

/// The URL scheme of plugin files.
pub const SCHEME: &str = "hindsight-plugin";

/// The content security policy sent with every plugin file: a plugin can use only what is in its
/// package, and cannot send anything out.
const CSP: &[&str] = &[
    // Anything not listed below cannot be loaded
    "default-src 'none'",
    // Run only .js files from the plugin package. No inline scripts, no attributes such as
    // onerror=, no eval: HTML put into the page cannot run as a script
    "script-src 'self'",
    // Workers can use only scripts from the plugin package
    "worker-src 'self'",
    // Allow inline styles, which UI libraries often use. To send data out, styles would have to
    // load outside images or fonts, and both are blocked below
    "style-src 'self' 'unsafe-inline'",
    // No outside images: an image URL can carry data out. data: and blob: are made by the page
    "img-src 'self' data: blob:",
    // Fonts only from the plugin package, or written in the page
    "font-src 'self' data:",
    // Sounds only from the plugin package, or made by the page, such as an alert sound
    "media-src 'self' data: blob:",
    // fetch, XHR, WebSocket and sendBeacon can request only files in the plugin package, not
    // outside URLs or other plugins
    "connect-src 'self'",
    // No other web pages inside a plugin page
    "frame-src 'none'",
    // No <object> or <embed>
    "object-src 'none'",
    // No <base> to change where relative paths point
    "base-uri 'none'",
    // No form submits: a form submit can also send data out
    "form-action 'none'",
];

/// The address of Hindsight's own pages.
pub(super) const HINDSIGHT_ORIGIN: &str = if tauri::is_dev() {
    // With `tauri dev`, Hindsight's pages come from `devUrl` in tauri.conf.json
    "http://localhost:41420"
} else if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

/// The launcher page. The CSP does not allow inline scripts, so the script is in `_launcher.js`.
const LAUNCHER_HTML: &str = r#"<!doctype html><script src="_launcher.js"></script>"#;

/// The launcher script: runs [`WORKER_JS`] in a Worker, and passes messages between the Worker
/// and the parent page.
///
/// Heartbeat: pings the Worker every 10 seconds, and stops it when two pings in a row get no
/// reply, so that a plugin stuck in a loop does not keep a CPU core busy. 停下以后告诉父页面。
///
/// `"*"`: the parent page can only be a Hindsight page, see [`ok`].
const LAUNCHER_JS: &str = r#"const worker = new Worker("_worker.js");
let missed = 0;
const heartbeat = setInterval(() => {
  if (missed === 2) {
    clearInterval(heartbeat);
    worker.terminate();
    console.warn("plugin stopped: no reply to two pings");
    parent.postMessage({ stopped: true }, "*");
    return;
  }
  missed += 1;
  worker.postMessage({ ping: true });
}, 10000);
worker.onmessage = (e) => {
  if (e.data?.pong) {
    missed = 0;
    return;
  }
  parent.postMessage(e.data, "*");
};
onmessage = (e) => {
  if (e.source === parent) worker.postMessage(e.data);
};
"#;

/// The script that runs first in the Worker: defines `hindsight.call()` and `hindsight.on()`, then
/// loads the plugin's `background.js`.
///
/// It also answers the launcher's pings, see [`LAUNCHER_JS`].
///
/// Wrapped in `{}` so that its variable names do not clash with those in `background.js`.
const WORKER_JS: &str = r#"{
  const pending = new Map();
  const listeners = new Map();
  let nextId = 1;
  self.hindsight = {
    call(method, params) {
      const id = nextId++;
      postMessage({ id, method, params });
      return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
    },
    on(name, callback) {
      if (!listeners.has(name)) listeners.set(name, []);
      listeners.get(name).push(callback);
    },
  };
  addEventListener("message", (e) => {
    if (e.data?.ping) {
      postMessage({ pong: true });
      return;
    }
    if (typeof e.data?.eventName === "string") {
      for (const callback of listeners.get(e.data.eventName) ?? []) {
        try {
          callback(e.data.data);
        } catch (err) {
          console.error(err);
        }
      }
      return;
    }
    const call = pending.get(e.data?.id);
    if (!call) return;
    pending.delete(e.data.id);
    if ("error" in e.data) call.reject(new Error(e.data.error));
    else call.resolve(e.data.result);
  });
}
importScripts("background.js");
"#;

/// The script plugin pages use to ask for data. A page loads it with
/// `<script src="../_page.js"></script>`, and can then call `hindsight.call()`.
///
/// The plugin page is inside a Hindsight page and cannot call Rust itself. It sends each request
/// to the parent page with postMessage. The forwarder there calls `plugin_call` and sends the
/// result back. `"*"` does not limit which page gets the message: the plugin page can only be
/// inside a Hindsight page (see [`ok`]), so the receiver is always Hindsight.
///
/// There is no `hindsight.on()`: events go only to the background script.
///
/// Wrapped in `{}`: all scripts on a page share top-level names. Without the braces, a `pending`
/// in the plugin's own script would be a duplicate declaration.
const PAGE_JS: &str = r#"{
  const pending = new Map();
  let nextId = 1;
  window.hindsight = {
    call(method, params) {
      const id = nextId++;
      parent.postMessage({ id, method, params }, "*");
      return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
    },
  };
  addEventListener("message", (e) => {
    if (e.source !== parent) return;
    const call = pending.get(e.data?.id);
    if (!call) return;
    pending.delete(e.data.id);
    if ("error" in e.data) call.reject(new Error(e.data.error));
    else call.resolve(e.data.result);
  });
}
"#;

/// Returns the launcher URL of version `version` of plugin `id`.
pub fn launcher_url(id: &str, version: &str) -> String {
    url(id, version, "_launcher.html")
}

/// Returns the page URL of version `version` of plugin `id`.
pub fn page_url(id: &str, version: &str) -> String {
    url(id, version, "ui/index.html")
}

fn url(id: &str, version: &str, file: &str) -> String {
    if cfg!(windows) {
        format!("http://{SCHEME}.{id}.localhost/{version}/{file}")
    } else {
        format!("{SCHEME}://{id}.localhost/{version}/{file}")
    }
}

/// Reads a file from the plugin library `plugins_dir` by URL and returns the response for the
/// WebView.
///
/// Returns 404 if the URL is wrong, the path leaves the version folder, or the file does not
/// exist. The reason is written only to the debug log.
pub fn read(plugins_dir: &Path, uri: &Uri) -> Response<Vec<u8>> {
    let data = file_path(plugins_dir, uri).and_then(|path| match hindsight_file(uri) {
        Some(text) => Ok(text.into()),
        None => Ok(fs::read(path)?),
    });
    match data {
        Ok(data) => ok(data, uri.path()),
        Err(e) => {
            log::debug!("plugins: cannot read {uri}: {e}");
            not_found()
        }
    }
}

/// Returns the file's contents when the URL points to a file Hindsight writes itself. For any
/// other file it returns `None`, and the caller reads the file from the plugin package.
fn hindsight_file(uri: &Uri) -> Option<&'static str> {
    let (_version, file) = uri.path().strip_prefix('/')?.split_once('/')?;
    match file {
        "_launcher.html" => Some(LAUNCHER_HTML),
        "_launcher.js" => Some(LAUNCHER_JS),
        "_worker.js" => Some(WORKER_JS),
        "_page.js" => Some(PAGE_JS),
        _ => None,
    }
}

/// Resolves a plugin file URL to a local path.
///
/// The resulting path has the form
/// `<plugins_dir>/<plugin-id>/<version>/<relative-path>`. Percent-encoded sequences
/// in the URL path (such as `%20`) are not decoded.
///
/// # Errors
///
/// Returns an error if the URL is invalid or the path is not contained within the
/// plugin's version directory.
///
/// # Examples
///
/// ```text
/// plugins_dir: /data/plugins
///
/// URI:    hindsight-plugin://hindsight-focus.localhost/0.1.0/ui/index.html
/// Result:   /data/plugins/hindsight-focus/0.1.0/ui/index.html
///
/// URI:    hindsight-plugin://hindsight-focus.localhost/0.1.0/../data.sqlite
/// Result: error: path escapes the version directory
/// ```
fn file_path(plugins_dir: &Path, uri: &Uri) -> Result<PathBuf> {
    let bad = || Error::InvalidInputDyn(format!("plugin file URL: {uri}"));
    let id = uri
        .host()
        .and_then(|host| host.strip_suffix(".localhost"))
        .ok_or_else(bad)?;
    check_id(id)?;
    let (version, file) = uri
        .path()
        .strip_prefix('/')
        .and_then(|path| path.split_once('/'))
        .ok_or_else(bad)?;
    check_version(version)?;
    // The file must be inside the version folder: no `..`, root or drive letter
    let file = Path::new(file);
    if file.as_os_str().is_empty() || !file.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(bad());
    }
    Ok(plugins_dir.join(id).join(version).join(file))
}

/// Builds a 200 response: sets the type from the content and the extension, adds utf-8 for text,
/// and adds the CSP.
fn ok(data: Vec<u8>, path: &str) -> Response<Vec<u8>> {
    let mime = MimeType::parse_with_fallback(&data, path, MimeType::OctetStream);
    let content_type = if mime.starts_with("text/") {
        format!("{mime}; charset=utf-8")
    } else {
        mime
    };
    // Only Hindsight's own pages can put a plugin page in an iframe
    let csp = format!("{}; frame-ancestors {HINDSIGHT_ORIGIN}", CSP.join("; "));
    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_SECURITY_POLICY, csp)
        .body(data)
        // The header values are one of a few fixed strings. The URL and the file content are never
        // copied into them, so this cannot fail. If it did, it still must not panic: in release
        // builds a panic ends the whole app
        .unwrap_or_else(|_| not_found())
}

fn not_found() -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::TempDir;

    /// 插件库里放 hindsight-focus 0.1.0 的几个文件，版本文件夹外面放它的 data.sqlite。
    fn library(dir: &TempDir) -> PathBuf {
        let version = dir.plugins().join("hindsight-focus").join("0.1.0");
        fs::create_dir_all(version.join("ui")).unwrap();
        fs::write(version.join("background.js"), "tick()").unwrap();
        fs::write(version.join("ui").join("index.html"), "<p>专注</p>").unwrap();
        fs::write(version.join("LICENSE"), "MIT").unwrap();
        fs::write(
            dir.plugins().join("hindsight-focus").join("data.sqlite"),
            "secret",
        )
        .unwrap();
        dir.plugins()
    }

    fn get(plugins: &Path, uri: &str) -> Response<Vec<u8>> {
        read(plugins, &uri.parse().unwrap())
    }

    /// 读得到的：内容原样交回；js、html 按扩展名标类型并带 utf-8，不认识的标成二进制；都带 CSP。
    #[test]
    fn reads_files_with_type_and_csp() {
        let dir = TempDir::new();
        let plugins = library(&dir);
        for (path, body, content_type) in [
            (
                "/0.1.0/background.js",
                "tick()",
                "text/javascript; charset=utf-8",
            ),
            (
                "/0.1.0/ui/index.html",
                "<p>专注</p>",
                "text/html; charset=utf-8",
            ),
            ("/0.1.0/LICENSE", "MIT", "application/octet-stream"),
        ] {
            let uri = format!("hindsight-plugin://hindsight-focus.localhost{path}");
            let response = get(&plugins, &uri);

            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(response.body(), body.as_bytes(), "{path}");
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                content_type,
                "{path}"
            );
            let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap();
            assert!(csp.contains("connect-src 'self'"), "{path}: {csp}");
            assert!(
                csp.ends_with(&format!("frame-ancestors {HINDSIGHT_ORIGIN}")),
                "{path}: {csp}"
            );
        }
    }

    /// 版本文件夹下的启动器三个文件和 `_page.js` 用 Rust 里写的内容，插件包里的同名文件不算。
    /// 都带 CSP。
    #[test]
    fn hindsight_files_come_from_hindsight() {
        let dir = TempDir::new();
        let plugins = library(&dir);
        let version = plugins.join("hindsight-focus").join("0.1.0");
        fs::write(version.join("_launcher.js"), "fake()").unwrap();
        for (path, body, content_type) in [
            (
                "/0.1.0/_launcher.html",
                LAUNCHER_HTML,
                "text/html; charset=utf-8",
            ),
            (
                "/0.1.0/_launcher.js",
                LAUNCHER_JS,
                "text/javascript; charset=utf-8",
            ),
            (
                "/0.1.0/_worker.js",
                WORKER_JS,
                "text/javascript; charset=utf-8",
            ),
            ("/0.1.0/_page.js", PAGE_JS, "text/javascript; charset=utf-8"),
        ] {
            let uri = format!("hindsight-plugin://hindsight-focus.localhost{path}");
            let response = get(&plugins, &uri);

            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(response.body(), body.as_bytes(), "{path}");
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                content_type,
                "{path}"
            );
            assert!(
                response
                    .headers()
                    .contains_key(header::CONTENT_SECURITY_POLICY),
                "{path}"
            );
        }
    }

    /// launcher_url、page_url 拼出来的 URL 读得到启动器和页面。Windows 上的 URL 要先经 Tauri
    /// 换回来，这里不测。
    #[cfg(not(windows))]
    #[test]
    fn urls_read_the_launcher_and_the_page() {
        let dir = TempDir::new();
        let plugins = library(&dir);
        let response = get(&plugins, &launcher_url("hindsight-focus", "0.1.0"));
        assert_eq!(response.body(), LAUNCHER_HTML.as_bytes());
        let response = get(&plugins, &page_url("hindsight-focus", "0.1.0"));
        assert_eq!(response.body(), "<p>专注</p>".as_bytes());
    }

    /// 读不到的一律 404：主机名、版本不对，跑出版本文件夹，文件不存在。
    #[test]
    fn rejects_bad_urls() {
        let dir = TempDir::new();
        let plugins = library(&dir);
        for uri in [
            // 主机名没有 .localhost
            "hindsight-plugin://hindsight-focus/0.1.0/background.js",
            // 版本不是 数字.数字.数字
            "hindsight-plugin://hindsight-focus.localhost/latest/background.js",
            "hindsight-plugin://hindsight-focus.localhost/../hindsight-focus/data.sqlite",
            // 版本文件夹外面的 data.sqlite
            "hindsight-plugin://hindsight-focus.localhost/0.1.0/../data.sqlite",
            "hindsight-plugin://hindsight-focus.localhost/0.1.0/%2e%2e/data.sqlite",
            // 没有文件名、文件不存在、是文件夹
            "hindsight-plugin://hindsight-focus.localhost/0.1.0/",
            "hindsight-plugin://hindsight-focus.localhost/0.1.0/missing.js",
            "hindsight-plugin://hindsight-focus.localhost/0.1.0/ui",
            // 没装的插件
            "hindsight-plugin://notes.localhost/0.1.0/background.js",
        ] {
            assert_eq!(get(&plugins, uri).status(), StatusCode::NOT_FOUND, "{uri}");
        }
    }
}
