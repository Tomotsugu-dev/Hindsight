//! Downloads a website's icon (ADR-0014): `/favicon.ico` first, then the icon address the home
//! page declares. The result is a PNG no larger than 32×32.

use std::future::Future;
use std::io::Cursor;
use std::time::Duration;

use image::ImageFormat;
use reqwest::Url;

use crate::error::{Error, Result};

/// Icons are drawn at 16 px; 32 leaves room for Retina screens.
const ICON_PX: u32 = 32;
/// Bodies are cut off here. An icon, or the `<head>` of a home page, fits well within it.
const MAX_BODY: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_REDIRECTS: usize = 5;

/// One answered GET.
pub(crate) struct Fetched {
    /// The address after redirects; a relative icon address in the page is resolved against it.
    pub url: Url,
    pub status: u16,
    pub body: Vec<u8>,
}

/// The requests this module sends; tests use a fake.
pub(crate) trait Http: Send + Sync {
    /// `None` when there is no answer: a timeout, a refused connection, or a redirect to a
    /// local address.
    fn get(&self, url: Url) -> impl Future<Output = Option<Fetched>> + Send;
}

pub(crate) struct ReqwestHttp(reqwest::Client);

impl ReqwestHttp {
    pub(crate) fn new() -> Result<Self> {
        // A website could redirect to a local address; those are never requested.
        let redirect = reqwest::redirect::Policy::custom(|attempt| {
            let local = attempt.url().host_str().is_none_or(is_local);
            if local || attempt.previous().len() >= MAX_REDIRECTS {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(redirect)
            .user_agent(concat!("Hindsight/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Other(format!("website icon HTTP client: {e}")))?;
        Ok(Self(client))
    }
}

impl Http for ReqwestHttp {
    async fn get(&self, url: Url) -> Option<Fetched> {
        let mut resp = self.0.get(url).send().await.ok()?;
        let url = resp.url().clone();
        let status = resp.status().as_u16();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.ok()? {
            body.extend_from_slice(&chunk);
            if body.len() >= MAX_BODY {
                body.truncate(MAX_BODY);
                break;
            }
        }
        Some(Fetched { url, status, body })
    }
}

/// Addresses that exist only on this machine or the local network. They are never requested.
/// A company intranet name that looks like a normal domain cannot be told apart.
pub(crate) fn is_local(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    !host.contains('.')
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.parse::<std::net::IpAddr>().is_ok()
}

/// Finds and downloads the icon of website `host`, as a PNG. Tries `/favicon.ico` first,
/// then the icon the home page declares. `None` when neither gives an image.
pub(crate) async fn fetch_site_icon(http: &impl Http, host: &str) -> Option<Vec<u8>> {
    // First try the standard location `/favicon.ico`.
    let home = Url::parse(&format!("https://{host}/")).ok()?;
    if let Some(png) = fetch_icon_at_url(http, home.join("/favicon.ico").ok()?).await {
        return Some(png);
    }
    // `/favicon.ico` failed: fetch the home page and read the icon address it declares.
    let page = http.get(home).await?;
    if !(200..300).contains(&page.status) {
        return None;
    }
    let href = icon_href(&String::from_utf8_lossy(&page.body))?;
    // Resolved against the address after redirects: a sign-in page may declare the icon.
    fetch_icon_at_url(http, page.url.join(&href).ok()?).await
}

/// Downloads the single address `url` and converts it to a PNG. `None` for a local or
/// non-web address, a failed request, or content that is not a supported image.
async fn fetch_icon_at_url(http: &impl Http, url: Url) -> Option<Vec<u8>> {
    let web = matches!(url.scheme(), "https" | "http");
    if !web || url.host_str().is_none_or(is_local) {
        return None;
    }
    let got = http.get(url).await?;
    if !(200..300).contains(&got.status) {
        return None;
    }
    to_png(&got.body)
}

/// The icon address a page declares in `<link rel="icon" href="…">`, as written in the page.
/// `icon` and `shortcut icon` come before `apple-touch-icon`. SVG is skipped: it cannot be
/// decoded.
fn icon_href(html: &str) -> Option<String> {
    // ASCII lowercasing keeps byte offsets, so positions found in `lower` work on `html`.
    let lower = html.to_ascii_lowercase();
    let mut touch_icon = None;
    let mut from = 0;
    while let Some(found) = lower[from..].find("<link") {
        let start = from + found;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let tag = &html[start..end];
        from = end;

        let Some(href) = attr(tag, "href") else {
            continue;
        };
        let svg = href
            .to_ascii_lowercase()
            .split(['?', '#'])
            .next()?
            .ends_with(".svg")
            || attr(tag, "type").is_some_and(|t| t.eq_ignore_ascii_case("image/svg+xml"));
        if svg {
            continue;
        }
        // `rel` can hold several words, such as `shortcut icon`. A word `icon` is the best
        // match and returns at once; `apple-touch-icon` is only kept as a fallback.
        let rel = attr(tag, "rel").unwrap_or_default().to_ascii_lowercase();
        let mut tokens = rel.split_ascii_whitespace();
        if tokens.clone().any(|t| t == "icon") {
            return Some(href);
        }
        if touch_icon.is_none() && tokens.any(|t| t.starts_with("apple-touch-icon")) {
            touch_icon = Some(href);
        }
    }
    touch_icon
}

/// Reads one attribute from a single HTML tag, such as `href` from
/// `<link rel="icon" href="/a.png">`, which gives `/a.png`.
///
/// `tag` is the text of one tag, `name` the attribute to read (case-insensitive). The value may
/// be in double quotes, single quotes, or bare. `&amp;` in it is turned into `&`. `None` when the
/// tag has no such attribute.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        // The name must stand on its own: `rel`, not `data-rel`; `href`, not `hreflang`.
        let rest = lower[from..].trim_start();
        if !lower[..at].ends_with(|c: char| c.is_ascii_whitespace()) || !rest.starts_with('=') {
            continue;
        }
        let value = tag[tag.len() - rest.len() + 1..].trim_start();
        let value = match value.chars().next()? {
            q @ ('"' | '\'') => value[1..].split(q).next()?,
            _ => value
                .split(|c: char| c.is_ascii_whitespace() || c == '>')
                .next()?,
        };
        return Some(value.replace("&amp;", "&"));
    }
    None
}

/// Decodes ICO, PNG or JPEG and scales it down to fit 32×32. `None` for anything else, such as
/// an HTML page sent with status 200.
fn to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = crate::icons::shrink_to_fit(image::load_from_memory(bytes).ok()?, ICON_PX);
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .ok()?;
    Some(out)
}

/// 测试用的假网络和图片。
#[cfg(test)]
pub(super) mod fake {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// 按网址返回事先放好的回答，并记下请求过哪些网址。
    #[derive(Default)]
    pub(in crate::icons::site) struct FakeHttp {
        /// 网址 → (跳转后的网址, 状态码, 内容)
        answers: HashMap<String, (String, u16, Vec<u8>)>,
        requested: Mutex<Vec<String>>,
    }

    impl FakeHttp {
        pub(in crate::icons::site) fn answer(
            mut self,
            url: &str,
            status: u16,
            body: impl Into<Vec<u8>>,
        ) -> Self {
            self.answers
                .insert(url.into(), (url.into(), status, body.into()));
            self
        }

        /// `url` 跳到 `to`，回答的是 `to` 的内容。
        pub(in crate::icons::site) fn redirect(
            mut self,
            url: &str,
            to: &str,
            status: u16,
            body: impl Into<Vec<u8>>,
        ) -> Self {
            self.answers
                .insert(url.into(), (to.into(), status, body.into()));
            self
        }

        pub(in crate::icons::site) fn requested(&self) -> Vec<String> {
            self.requested.lock().unwrap().clone()
        }
    }

    impl Http for FakeHttp {
        async fn get(&self, url: Url) -> Option<Fetched> {
            self.requested.lock().unwrap().push(url.to_string());
            let (to, status, body) = self.answers.get(url.as_str())?.clone();
            Some(Fetched {
                url: Url::parse(&to).unwrap(),
                status,
                body,
            })
        }
    }

    pub(in crate::icons::site) fn png(px: u32) -> Vec<u8> {
        encode(px, ImageFormat::Png)
    }

    pub(in crate::icons::site) fn ico(px: u32) -> Vec<u8> {
        encode(px, ImageFormat::Ico)
    }

    fn encode(px: u32, format: ImageFormat) -> Vec<u8> {
        let img = image::DynamicImage::new_rgba8(px, px);
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), format).unwrap();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{ico, png, FakeHttp};
    use super::*;

    fn size(png: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(png).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn local_addresses() {
        for host in [
            "localhost",
            "router",
            "dev.localhost",
            "printer.local",
            "192.168.1.10",
            "::1",
        ] {
            assert!(is_local(host), "{host} 应算本地地址");
        }
        for host in ["github.com", "live.bilibili.com", "bbc.co.uk"] {
            assert!(!is_local(host), "{host} 不是本地地址");
        }
    }

    #[test]
    fn icon_href_reads_the_declared_icon() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="/a.css">
            <link rel="apple-touch-icon" href="/touch.png">
            <link rel="shortcut icon" href="https://static.zhihu.com/favicon.ico?v=1&amp;x=2">
        </head></html>"#;
        assert_eq!(
            icon_href(html).as_deref(),
            Some("https://static.zhihu.com/favicon.ico?v=1&x=2"),
            "icon 优先于 apple-touch-icon，&amp; 要还原"
        );
    }

    #[test]
    fn icon_href_skips_svg_and_falls_back_to_touch_icon() {
        let html = r#"<LINK REL=icon TYPE="image/svg+xml" HREF=/logo.svg>
            <link href='/favicon.svg?v=3' rel='icon'>
            <link data-rel="icon" rel="apple-touch-icon" href="/touch.png">"#;
        assert_eq!(icon_href(html).as_deref(), Some("/touch.png"));
    }

    #[test]
    fn icon_href_none_without_icon_links() {
        assert_eq!(icon_href(r#"<link rel="stylesheet" href="/a.css">"#), None);
        assert_eq!(icon_href("not html"), None);
    }

    #[test]
    fn to_png_scales_large_icons_and_rejects_html() {
        assert_eq!(size(&to_png(&png(256)).unwrap()), (32, 32));
        assert_eq!(size(&to_png(&ico(48)).unwrap()), (32, 32));
        assert_eq!(size(&to_png(&png(16)).unwrap()), (16, 16), "小图不放大");
        assert!(to_png(b"<!doctype html><title>Sign in</title>").is_none());
    }

    #[tokio::test]
    async fn favicon_ico_is_enough() {
        let http = FakeHttp::default().answer("https://github.com/favicon.ico", 200, ico(32));
        assert!(fetch_site_icon(&http, "github.com").await.is_some());
        assert_eq!(http.requested(), ["https://github.com/favicon.ico"]);
    }

    /// notion.so 那种：根目录没有 favicon.ico，图标在首页里声明。
    #[tokio::test]
    async fn falls_back_to_the_icon_the_home_page_declares() {
        let http = FakeHttp::default()
            .answer("https://notion.so/favicon.ico", 404, "")
            .answer(
                "https://notion.so/",
                200,
                r#"<link rel="icon" href="/images/favicon.png">"#,
            )
            .answer("https://notion.so/images/favicon.png", 200, png(64));
        assert!(fetch_site_icon(&http, "notion.so").await.is_some());
    }

    /// zhihu.com 那种：favicon.ico 返回 200 但内容是 HTML；首页跳到登录页，
    /// 相对地址要按跳转后的网址补全。
    #[tokio::test]
    async fn html_with_status_200_is_not_an_icon() {
        let http = FakeHttp::default()
            .answer("https://zhihu.com/favicon.ico", 200, "<html>sign in</html>")
            .redirect(
                "https://zhihu.com/",
                "https://www.zhihu.com/signin",
                200,
                r#"<link rel="shortcut icon" href="static/favicon.ico">"#,
            )
            .answer("https://www.zhihu.com/static/favicon.ico", 200, ico(32));
        assert!(fetch_site_icon(&http, "zhihu.com").await.is_some());
    }

    /// openai.com 那种：都被拦下，拿不到。
    #[tokio::test]
    async fn blocked_everywhere_gives_none() {
        let http = FakeHttp::default()
            .answer("https://openai.com/favicon.ico", 403, "blocked")
            .answer("https://openai.com/", 403, "blocked");
        assert!(fetch_site_icon(&http, "openai.com").await.is_none());
    }

    /// 首页声明的图标在本地地址上：不去请求。
    #[tokio::test]
    async fn icon_on_a_local_address_is_not_requested() {
        let http = FakeHttp::default()
            .answer("https://example.com/favicon.ico", 404, "")
            .answer(
                "https://example.com/",
                200,
                r#"<link rel="icon" href="http://192.168.1.1/x.ico">"#,
            );
        assert!(fetch_site_icon(&http, "example.com").await.is_none());
        assert!(!http.requested().iter().any(|u| u.contains("192.168.1.1")));
    }
}
