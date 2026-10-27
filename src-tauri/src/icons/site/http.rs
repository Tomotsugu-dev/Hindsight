//! Sends the requests: [`ReqwestHttp`] implements [`Http`] with reqwest. Local addresses are
//! never requested.

use std::time::Duration;

use reqwest::Url;

use super::{Fetched, Http};
use crate::error::{Error, Result};

/// Bodies are cut off here. An icon, or the `<head>` of a home page, fits well within it.
const MAX_BODY: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_REDIRECTS: usize = 5;

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
pub(super) fn is_local(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    !host.contains('.')
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.parse::<std::net::IpAddr>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
