//! The WebDAV backend's pure half: nothing here talks to a server. Paths
//! follow the layout of ADR-0007 §1 and are relative to the sync root.

use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::NsReader;

use crate::error::{Error, Result};
use crate::sync::file_name::FileName;

// ─────────────── Directory Layout (ADR-0007 §1) ───────────────
//
// Paths are relative to the sync root (`/hindsight/` on the server); the
// client prepends the root. Which names exist, and which kinds are day
// files, is `file_name.rs`'s business; this module only places them.
//
//   device.<id>.<kind>.<date>.ndjson  ↔  <id>/<kind>/<year>/<date>.ndjson
//   device.<id>.<kind>.json           ↔  <id>/<kind>.json

/// Converts the engine's flat file name into a WebDAV path.
/// Returns `None` for names that do not match the sync file format.
pub(crate) fn flat_name_to_path(flat_name: &str) -> Option<String> {
    let FileName { device_id, kind } = FileName::parse(flat_name)?;
    if !is_valid_path(&device_id) {
        return None;
    }
    let segment = kind.segment();
    match kind.date() {
        // <id>/<kind>/<year>/<date>.ndjson
        Some(date) => Some(format!(
            "{device_id}/{segment}/{}/{date}.ndjson",
            date.format("%Y")
        )),
        // <id>/<kind>.json
        None => Some(format!("{device_id}/{segment}.json")),
    }
}

/// Converts a WebDAV path back into the engine's flat file name.
/// Returns `None` for paths that do not correspond to the sync file layout.
pub(crate) fn path_to_flat_name(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    let flat_name = match parts.as_slice() {
        // <id>/<kind>/<year>/<date>.ndjson  ->  device.<id>.<kind>.<date>.ndjson
        [id, kind, year, file] if is_valid_path(id) => {
            let date = file.strip_suffix(".ndjson")?;
            if date.get(..4)? != *year {
                return None;
            }
            format!("device.{id}.{kind}.{date}.ndjson")
        }
        // <id>/<kind>.json  ->  device.<id>.<kind>.json
        [id, file] if is_valid_path(id) => {
            let kind = file.strip_suffix(".json")?;
            format!("device.{id}.{kind}.json")
        }
        _ => return None,
    };
    // The tree may hold files the engine never wrote; only names it knows come back.
    FileName::parse(&flat_name).map(|_| flat_name)
}

/// Returns a temporary upload path in the same directory as the final path.
pub(crate) fn temporary_upload_path(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/.tmp-{file}"),
        None => format!(".tmp-{path}"),
    }
}

/// Return true if the string is a valid path segment:
/// non-empty and does not contain '.' or '/'.
fn is_valid_path(s: &str) -> bool {
    !s.is_empty() && !s.contains(['.', '/'])
}

// ─────────────── Date Handling ───────────────

/// Converts WebDAV's `getlastmodified` property from an HTTP-date string
/// to RFC3339, the timestamp format used by Hindsight sync cursors.
///
/// Example: `Sun, 06 Nov 1994 08:49:37 GMT` becomes
/// `1994-11-06T08:49:37Z`.
pub(crate) fn http_date_to_rfc3339(s: &str) -> Option<String> {
    let s = s.trim();
    let utc = match chrono::DateTime::parse_from_rfc2822(s) {
        Ok(t) => t.with_timezone(&chrono::Utc),
        // RFC 850 and asctime are also HTTP-date formats and use GMT.
        Err(_) => {
            let single_spaced = s.split_whitespace().collect::<Vec<_>>().join(" ");
            chrono::NaiveDateTime::parse_from_str(&single_spaced, "%A, %d-%b-%y %H:%M:%S GMT")
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(&single_spaced, "%a %b %d %H:%M:%S %Y")
                })
                .ok()?
                .and_utc()
        }
    };
    Some(utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

// ─────────────── PROPFIND ───────────────

/// One file or directory returned by a WebDAV `PROPFIND` request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DavEntry {
    /// The path returned by the server, usually percent-encoded.
    pub href: String,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// The last modified time in RFC3339, or `None` if it is missing or invalid.
    pub modified: Option<String>,
    /// The file size in bytes, or `None` when the server does not provide it.
    pub size: Option<u64>,
}

/// The properties read from each `PROPFIND` response.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Href,
    Modified,
    Size,
}

impl Field {
    fn from_xml_name(name: &[u8]) -> Option<Self> {
        match name {
            b"href" => Some(Self::Href),
            b"getlastmodified" => Some(Self::Modified),
            b"getcontentlength" => Some(Self::Size),
            _ => None,
        }
    }

    fn store_value(self, entry: &mut DavEntry, value: &str) {
        match self {
            Self::Href => entry.href = value.to_string(),
            Self::Modified => entry.modified = http_date_to_rfc3339(value),
            Self::Size => entry.size = value.parse().ok(),
        }
    }
}

/// Reads a `PROPFIND` multistatus body: one [`DavEntry`] per `<response>`.
/// A body that is not a multistatus, or is cut short, is an error, not an
/// empty list.
pub(crate) fn parse_multistatus(xml: &[u8]) -> Result<Vec<DavEntry>> {
    let mut reader = NsReader::from_reader(xml);
    let mut entries = Vec::new();
    let mut current: Option<DavEntry> = None;
    let mut field: Option<Field> = None;
    let mut text = String::new();
    let mut saw_root = false;
    let mut depth = 0usize;

    loop {
        let (ns, event) = reader.read_resolved_event().map_err(parse_err)?;
        let is_dav: bool = matches!(&ns, ResolveResult::Bound(Namespace(n)) if *n == b"DAV:");
        match event {
            Event::Start(e) => {
                depth += 1;
                if !is_dav {
                    continue;
                }
                match e.local_name().as_ref() {
                    b"multistatus" => saw_root = true,
                    b"response" => current = Some(DavEntry::default()),
                    b"collection" => {
                        if let Some(c) = current.as_mut() {
                            c.is_dir = true;
                        }
                    }
                    name => {
                        if current.is_some() {
                            if let Some(f) = Field::from_xml_name(name) {
                                field = Some(f);
                                text.clear();
                            }
                        }
                    }
                }
            }
            Event::Empty(e) => {
                if is_dav && e.local_name().as_ref() == b"collection" {
                    if let Some(c) = current.as_mut() {
                        c.is_dir = true;
                    }
                }
            }
            Event::Text(t) => {
                if field.is_some() {
                    text.push_str(&t.xml_content().map_err(parse_err)?);
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                if !is_dav {
                    continue;
                }
                match e.local_name().as_ref() {
                    b"response" => entries.extend(current.take()),
                    name => {
                        if let Some(f) = Field::from_xml_name(name) {
                            if field == Some(f) {
                                if let Some(c) = current.as_mut() {
                                    f.store_value(c, text.trim());
                                }
                                field = None;
                            }
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !saw_root {
        return Err(parse_err("no DAV:multistatus element"));
    }
    if depth != 0 {
        return Err(parse_err("document ends inside an element"));
    }
    Ok(entries)
}

fn parse_err(e: impl std::fmt::Display) -> Error {
    Error::WebDavParse(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三种文件各落到布局里的位置；日文件按年份分目录。
    #[test]
    fn flat_name_to_path_places_each_kind() {
        assert_eq!(
            flat_name_to_path("device.abc.activities.2026-09-20.ndjson").as_deref(),
            Some("abc/activities/2026/2026-09-20.ndjson")
        );
        assert_eq!(
            flat_name_to_path("device.abc.memory.2026-09-20.ndjson").as_deref(),
            Some("abc/memory/2026/2026-09-20.ndjson")
        );
        assert_eq!(
            flat_name_to_path("device.abc.categories.json").as_deref(),
            Some("abc/categories.json")
        );
    }

    /// 引擎不写的名字不给路径：前缀不对、日期不成形、设备 id 带点或为空、不认识的种类。
    #[test]
    fn flat_name_to_path_rejects_names_the_engine_never_writes() {
        assert_eq!(flat_name_to_path("readme.txt"), None);
        assert_eq!(
            flat_name_to_path("device.abc.activities.today.ndjson"),
            None
        );
        assert_eq!(flat_name_to_path("device.a.b.categories.json"), None);
        assert_eq!(flat_name_to_path("device..categories.json"), None);
        assert_eq!(flat_name_to_path("device.abc.notes.json"), None);
    }

    /// 扁平名 → 路径 → 扁平名回到原样，每种文件各走一遍。
    #[test]
    fn path_to_flat_name_inverts_flat_name_to_path() {
        for name in [
            "device.abc.activities.2026-09-20.ndjson",
            "device.abc.memory.2026-09-20.ndjson",
            "device.abc.tombstone.json",
        ] {
            let path = flat_name_to_path(name).unwrap();
            assert_eq!(path_to_flat_name(&path).as_deref(), Some(name), "{name}");
        }
    }

    /// 列目录会看到、但不能当成同步文件的东西：临时名、目录本身、放错年份的日文件、
    /// 多余层级、不认识的种类。
    #[test]
    fn path_to_flat_name_skips_what_a_listing_must_ignore() {
        assert_eq!(
            path_to_flat_name("abc/activities/2026/.tmp-2026-09-20.ndjson"),
            None
        );
        assert_eq!(path_to_flat_name("abc/.tmp-categories.json"), None);
        assert_eq!(path_to_flat_name("abc/notes.json"), None);
        assert_eq!(
            path_to_flat_name("abc/categories/2026/2026-09-20.ndjson"),
            None
        );
        assert_eq!(path_to_flat_name("abc/activities/2026"), None);
        assert_eq!(
            path_to_flat_name("abc/activities/2025/2026-09-20.ndjson"),
            None
        );
        assert_eq!(
            path_to_flat_name("abc/activities/2026/x/2026-09-20.ndjson"),
            None
        );
    }

    /// 临时名留在同一目录，只改文件名。
    #[test]
    fn temporary_upload_path_stays_in_the_same_directory() {
        assert_eq!(
            temporary_upload_path("abc/activities/2026/2026-09-20.ndjson"),
            "abc/activities/2026/.tmp-2026-09-20.ndjson"
        );
    }

    /// RFC 7231 允许的三种写法都转成同一个 RFC3339；别的一律 None。
    #[test]
    fn http_date_accepts_all_three_rfc7231_forms() {
        let want = Some("1994-11-06T08:49:37Z");
        assert_eq!(
            http_date_to_rfc3339("Sun, 06 Nov 1994 08:49:37 GMT").as_deref(),
            want
        );
        assert_eq!(
            http_date_to_rfc3339("Sunday, 06-Nov-94 08:49:37 GMT").as_deref(),
            want
        );
        assert_eq!(
            http_date_to_rfc3339("Sun Nov  6 08:49:37 1994").as_deref(),
            want
        );
        assert_eq!(http_date_to_rfc3339("yesterday"), None);
    }

    const RFC4918_EXAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8" ?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/container/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
        <D:getlastmodified>Mon, 12 Jan 1998 09:25:56 GMT</D:getlastmodified>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/container/front.html</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype/>
        <D:getcontentlength>4525</D:getcontentlength>
        <D:getlastmodified>Mon, 12 Jan 1998 09:25:56 GMT</D:getlastmodified>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

    /// RFC 4918 的例子：目录靠 `<collection/>` 认出来，时间转成 RFC3339，大小是数字。
    #[test]
    fn parse_multistatus_reads_the_rfc4918_example() {
        let entries = parse_multistatus(RFC4918_EXAMPLE.as_bytes()).unwrap();
        assert_eq!(
            entries,
            vec![
                DavEntry {
                    href: "/container/".into(),
                    is_dir: true,
                    modified: Some("1998-01-12T09:25:56Z".into()),
                    size: None,
                },
                DavEntry {
                    href: "/container/front.html".into(),
                    is_dir: false,
                    modified: Some("1998-01-12T09:25:56Z".into()),
                    size: Some(4525),
                },
            ]
        );
    }

    const NEXTCLOUD_STYLE: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:oc="http://owncloud.org/ns">
  <d:response>
    <d:href>/remote.php/dav/files/alice/hindsight/abc/activities/2026/2026-09-20.ndjson</d:href>
    <d:propstat>
      <d:prop>
        <d:getlastmodified>Sun, 20 Sep 2026 08:12:03 GMT</d:getlastmodified>
        <d:getcontentlength>4821</d:getcontentlength>
        <d:resourcetype/>
        <oc:fileid>1234</oc:fileid>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
    <d:propstat>
      <d:prop>
        <d:quota-available-bytes/>
      </d:prop>
      <d:status>HTTP/1.1 404 Not Found</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

    /// 前缀叫 `d:` 也一样认；别的命名空间的属性和 404 那块 propstat 不干扰。
    #[test]
    fn parse_multistatus_goes_by_namespace_not_prefix() {
        let entries = parse_multistatus(NEXTCLOUD_STYLE.as_bytes()).unwrap();
        assert_eq!(
            entries,
            vec![DavEntry {
                href: "/remote.php/dav/files/alice/hindsight/abc/activities/2026/2026-09-20.ndjson"
                    .into(),
                is_dir: false,
                modified: Some("2026-09-20T08:12:03Z".into()),
                size: Some(4821),
            }]
        );
    }

    /// 默认命名空间（没有前缀）也认。
    #[test]
    fn parse_multistatus_accepts_the_default_namespace() {
        let xml = r#"<multistatus xmlns="DAV:"><response><href>/x/</href>
            <propstat><prop><resourcetype><collection/></resourcetype></prop>
            <status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;
        let entries = parse_multistatus(xml.as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].href, "/x/");
        assert!(entries[0].is_dir);
    }

    /// 不是 multistatus、或者半截断掉的响应要报错，不能变成「云上没文件」。
    #[test]
    fn parse_multistatus_rejects_what_is_not_a_listing() {
        assert!(matches!(
            parse_multistatus(b"<html><body>sign in</body></html>"),
            Err(Error::WebDavParse(_))
        ));
        assert!(matches!(
            parse_multistatus(br#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/a</D:href>"#),
            Err(Error::WebDavParse(_))
        ));
    }
}
