//! Navigation guard: a webview can open only Hindsight's own pages and plugin pages. A plugin page
//! that cannot go to another URL cannot put the data it read into a URL and take it off this
//! computer.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Runtime, Url};

use super::file_reader::{HINDSIGHT_ORIGIN, SCHEME};

/// Every navigation in every webview asks here first. It is canceled if not allowed.
pub fn guard<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("navigation-guard")
        // TODO: on Windows, wry passes only top-level navigations here, so plugin pages in iframes
        // are not guarded
        .on_navigation(|webview, url| {
            let allowed = allowed(url);
            if !allowed {
                log::warn!("plugins: blocked a navigation in {}", webview.label());
                log::debug!("plugins: blocked the navigation to {url}");
            }
            allowed
        })
        .build()
}

fn allowed(url: &Url) -> bool {
    // A blank page carries no data. Allowed, so that a new iframe is not blocked by mistake
    url.as_str() == "about:blank" || is_hindsight_page(url) || is_plugin_page(url)
}

fn is_hindsight_page(url: &Url) -> bool {
    let Ok(origin) = Url::parse(HINDSIGHT_ORIGIN) else {
        return false;
    };
    url.scheme() == origin.scheme()
        && url.host_str() == origin.host_str()
        && url.port_or_known_default() == origin.port_or_known_default()
}

/// The address of a plugin page, see [`super::file_reader::page_url`]. It has no port: an address
/// with a port is another service on this computer.
fn is_plugin_page(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    let ours = if cfg!(windows) {
        url.scheme() == "http" && host.starts_with(&format!("{SCHEME}."))
    } else {
        url.scheme() == SCHEME
    };
    ours && host.ends_with(".localhost") && url.port().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::file_reader::{launcher_url, page_url};

    fn allows(url: &str) -> bool {
        allowed(&url.parse().unwrap())
    }

    /// Hindsight 自己的页面、插件的页面和启动器、空白页都放行。
    #[test]
    fn allows_hindsight_and_plugin_pages() {
        for url in [
            format!("{HINDSIGHT_ORIGIN}/src/plugin-window/index.html"),
            page_url("hindsight-focus", "0.1.0"),
            launcher_url("hindsight-focus", "0.1.0"),
            "about:blank".into(),
        ] {
            assert!(allows(&url), "{url}");
        }
    }

    /// 别的一律拦下：外面的网站、本机别的端口、名字像 localhost 的外部主机、
    /// 借插件协议名的外部主机、带端口的插件地址。
    #[test]
    fn blocks_everything_else() {
        for url in [
            "https://example.com/?d=1",
            "http://127.0.0.1:8080/",
            "http://localhost:9999/",
            "https://tauri.localhost.example.com/",
            "data:text/html,hi",
            "hindsight-plugin://evil.example/",
            "http://hindsight-plugin.evil.example/",
            "hindsight-plugin://hindsight-focus.localhost:8080/0.1.0/ui/index.html",
            "http://hindsight-plugin.hindsight-focus.localhost:8080/0.1.0/ui/index.html",
        ] {
            assert!(!allows(url), "{url}");
        }
    }
}
