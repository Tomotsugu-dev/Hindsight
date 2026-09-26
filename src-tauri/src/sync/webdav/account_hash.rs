//! How this device recognizes a WebDAV account, and which prefix its server's
//! names carry in `sync_cursor` (ADR-0011 §5).

use sha2::{Digest, Sha256};
use url::Url;

use super::dav::parse_server_url;
use crate::account::WEBDAV_DB_PREFIX;
use crate::error::Result;

/// The account hash: `webdav-` followed by the first 16 hex characters of the
/// SHA-256 of the normalized address and user name, each written as a netstring,
/// `<byte length>:<content>,`. The lengths keep two different pairs from joining
/// into the same input.
pub(crate) fn account_hash(server_url: &str, user: &str) -> Result<String> {
    let input = netstring(&normalized_url(server_url)?) + &netstring(&user.trim().to_lowercase());
    let digest = Sha256::digest(input.as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{WEBDAV_DB_PREFIX}{hex}"))
}

/// The prefix of this server's names in `sync_cursor`, e.g.,
/// `webdav.dav.jianguoyun.com.`.
pub(crate) fn cursor_name_prefix(server_url: &str) -> Result<String> {
    Ok(format!("webdav.{}.", server_host(server_url)?))
}

/// `HTTPS://DAV.jianguoyun.com:443/dav/` gives `dav.jianguoyun.com`. Every
/// spelling of the same server gives the same value.
pub(crate) fn server_host(server_url: &str) -> Result<String> {
    Ok(host_and_port(&parse_server_url(server_url)?))
}

/// `30:https://dav.jianguoyun.com/dav,`. The length counts bytes, so it stays right
/// for non-ASCII user names.
fn netstring(s: &str) -> String {
    format!("{}:{s},", s.len())
}

/// The scheme and host in lowercase without the default port (parsing into `Url`
/// already does both), and the path without its trailing `/`.
fn normalized_url(server_url: &str) -> Result<String> {
    let url = parse_server_url(server_url)?;
    Ok(format!(
        "{}://{}{}",
        url.scheme(),
        host_and_port(&url),
        url.path().trim_end_matches('/')
    ))
}

/// The host, followed by the port only when it is not the default: `cloud.example.com:8443`.
fn host_and_port(url: &Url) -> String {
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0011 §5 的例子：同一个账号换种写法，账号哈希不变。
    #[test]
    fn spellings_of_one_account_share_a_hash() {
        assert_eq!(
            account_hash("HTTPS://DAV.jianguoyun.com:443/dav/", " You@Example.com ").unwrap(),
            account_hash("https://dav.jianguoyun.com/dav", "you@example.com").unwrap(),
        );
    }

    /// 输出钉死：规则一改，这条就失败，提醒已有用户会认不出自己的库。
    #[test]
    fn account_hash_is_fixed() {
        assert_eq!(
            account_hash("https://dav.jianguoyun.com/dav", "you@example.com").unwrap(),
            "webdav-2538daca8be4c388"
        );
    }

    /// 换服务器、换用户名，都是另一个账号。
    #[test]
    fn another_server_or_user_is_another_account() {
        let a = account_hash("https://dav.jianguoyun.com/dav", "you@example.com").unwrap();
        let other_server = account_hash(
            "https://cloud.example.com/remote.php/dav",
            "you@example.com",
        )
        .unwrap();
        let other_user = account_hash("https://dav.jianguoyun.com/dav", "me@example.com").unwrap();
        assert_ne!(a, other_server);
        assert_ne!(a, other_user);
    }

    /// 名字前缀：域名转小写，默认端口去掉，非默认端口留着。
    #[test]
    fn cursor_name_prefix_keeps_only_a_non_default_port() {
        assert_eq!(
            cursor_name_prefix("HTTPS://DAV.jianguoyun.com:443/dav/").unwrap(),
            "webdav.dav.jianguoyun.com."
        );
        assert_eq!(
            cursor_name_prefix("https://cloud.example.com:8443/remote.php/dav/").unwrap(),
            "webdav.cloud.example.com:8443."
        );
    }
}
