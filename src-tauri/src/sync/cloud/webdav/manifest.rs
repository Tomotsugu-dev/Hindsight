//! 每台设备在同步根目录下那份清单（ADR-0008）：文件名、内容格式、字节 ↔ 结构体。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Manifest Version.
pub(crate) const MANIFEST_VERSION: u32 = 1;

/// Reference: ADR-0008. Manifest structure for each device in the sync root directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Manifest {
    // Manifest version.
    pub version: u32,
    // Mapping from file paths to their last push count.
    pub files: BTreeMap<String, u64>,
}

impl Manifest {
    pub(crate) fn new() -> Self {
        Self {
            version: MANIFEST_VERSION,
            files: BTreeMap::new(),
        }
    }

    /// Parse a manifest from a byte slice.
    /// Returns an error if the JSON is invalid or the version is unrecognized.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let m: Manifest = serde_json::from_slice(bytes).map_err(|e| Error::SyncParse {
            kind: "manifest",
            source: e,
        })?;
        if m.version != MANIFEST_VERSION {
            return Err(Error::WebDavManifestVersion(m.version));
        }
        Ok(m)
    }

    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }

    /// Returns the highest push count in this manifest.
    /// Returns 0 if the manifest is empty.
    pub(crate) fn latest_push_count(&self) -> u64 {
        self.files.values().copied().max().unwrap_or(0)
    }
}

/// Manifest file name pattern: `manifest.<device-id>.json`.
pub(crate) fn manifest_path(device_id: &str) -> String {
    format!("manifest.{device_id}.json")
}

/// The inverse of [`manifest_path`]: returns `None` if the file name is not a manifest.
pub(crate) fn manifest_to_device_id(file_name: &str) -> Option<&str> {
    let id = file_name.strip_prefix("manifest.")?.strip_suffix(".json")?;
    (!id.is_empty() && !id.contains(['.', '/'])).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写出去再读回来一样；`files` 是 BTreeMap，序列化的顺序固定。
    #[test]
    fn manifest_roundtrips() {
        let mut m = Manifest::new();
        m.files.insert("categories.json".into(), 42);
        m.files
            .insert("activities/2026/2026-09-20.ndjson".into(), 41);
        let bytes = m.to_bytes().unwrap();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"version":1,"files":{"activities/2026/2026-09-20.ndjson":41,"categories.json":42}}"#
        );
        assert_eq!(Manifest::parse(&bytes).unwrap(), m);
        assert_eq!(m.latest_push_count(), 42);
        assert_eq!(Manifest::new().latest_push_count(), 0);
    }

    /// 不认识的 version 报错；坏 JSON 报错。
    #[test]
    fn manifest_rejects_unknown_version_and_garbage() {
        assert!(matches!(
            Manifest::parse(br#"{"version":2,"files":{}}"#),
            Err(Error::WebDavManifestVersion(2))
        ));
        assert!(matches!(
            Manifest::parse(b"<html>"),
            Err(Error::SyncParse {
                kind: "manifest",
                ..
            })
        ));
    }

    /// 清单文件名和设备 id 互换；别的文件名不算清单。
    #[test]
    fn manifest_path_roundtrips_with_device_id() {
        assert_eq!(manifest_path("abc"), "manifest.abc.json");
        assert_eq!(manifest_to_device_id("manifest.abc.json"), Some("abc"));
        assert_eq!(manifest_to_device_id("manifest..json"), None);
        assert_eq!(manifest_to_device_id("categories.json"), None);
        assert_eq!(manifest_to_device_id("manifest.a.b.json"), None);
    }
}
