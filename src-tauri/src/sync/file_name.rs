//! Naming rule of sync files: device and kind build a file name, and a file name
//! parses back into device and kind.
//!
//! A file name carries three things: the device that wrote the file, the kind of
//! file, and, for a day file, the date it covers. The format is part of the sync
//! protocol: a new version may add new file names, but must not change the format
//! of existing ones.
//!
//! ```text
//! device.<device-id>.<kind>.json                  one file per table
//! device.<device-id>.<kind>.<YYYY-MM-DD>.ndjson   one file per day
//! ```

use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileName {
    pub(crate) device_id: String,
    pub(crate) kind: FileKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum FileKind {
    /// One file per day: the date the rows belong to.
    Activities(NaiveDate),
    Categories,
    DeviceMeta,
    AppIcons,
    AppGroups,
    AppGroupMembers,
    /// A device asking every peer to drop everything it wrote before a given
    /// moment: `DELETE WHERE device_id = <owner> AND updated_at < clearedAt`.
    /// It exists because the engine only inserts and updates, so a row missing
    /// from a file carries no meaning — this is the only way to say "delete".
    Tombstone,
    /// Opt-in upload: AI generated summaries (merged in `datasets.rs`).
    AiSummaries,
    /// Opt-in upload: chat history.
    Chat,
    /// Opt-in upload: screen-memory full text, one file per day.
    Memory(NaiveDate),
}

/// Which dataset a file belongs to. A dataset is a group of files pulled together,
/// each with its own pull cursor; Core is always on, the other three are switched
/// on by the user in settings (ADR-0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Dataset {
    Core,
    AiSummaries,
    Chat,
    Memory,
}

impl Dataset {
    /// The dataset's name in `sync_cursor`: the `chat` in `push.chat`, and the key
    /// of WebDAV bookmarks. Users' databases hold it, so it must not change.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Dataset::Core => "core",
            Dataset::AiSummaries => "ai_summaries",
            Dataset::Chat => "chat",
            Dataset::Memory => "memory",
        }
    }
}

impl FileKind {
    /// Return string segment representing the file kind in the file name.
    pub(crate) fn segment(&self) -> &'static str {
        match self {
            FileKind::Activities(_) => "activities",
            FileKind::Categories => "categories",
            FileKind::DeviceMeta => "meta",
            FileKind::AppIcons => "icons",
            FileKind::AppGroups => "app_groups",
            FileKind::AppGroupMembers => "app_group_members",
            FileKind::Tombstone => "tombstone",
            FileKind::AiSummaries => "ai_summaries",
            FileKind::Chat => "chat",
            FileKind::Memory(_) => "memory",
        }
    }

    pub(crate) fn dataset(&self) -> Dataset {
        match self {
            FileKind::AiSummaries => Dataset::AiSummaries,
            FileKind::Chat => Dataset::Chat,
            FileKind::Memory(_) => Dataset::Memory,
            _ => Dataset::Core,
        }
    }

    /// The date a day file covers; `None` for the kinds that are one file per table.
    pub(crate) fn date(&self) -> Option<NaiveDate> {
        match self {
            FileKind::Activities(date) | FileKind::Memory(date) => Some(*date),
            _ => None,
        }
    }
}

impl FileName {
    pub(crate) fn to_file_name(&self) -> String {
        let id = &self.device_id;
        let kind = self.kind.segment();
        match &self.kind {
            FileKind::Activities(date) | FileKind::Memory(date) => {
                format!("device.{id}.{kind}.{date}.ndjson")
            }
            _ => format!("device.{id}.{kind}.json"),
        }
    }

    /// Reverse of [`Self::to_file_name`]. Returns `None` if the name is not recognized
    pub(crate) fn parse(name: &str) -> Option<Self> {
        let parts: Vec<&str> = name.split('.').collect();
        let (device_id, kind) = match parts.as_slice() {
            ["device", id, "activities", date, "ndjson"] => {
                (id, FileKind::Activities(parse_date(date)?))
            }
            ["device", id, "memory", date, "ndjson"] => (id, FileKind::Memory(parse_date(date)?)),
            ["device", id, segment, "json"] => {
                let kind = match *segment {
                    "categories" => FileKind::Categories,
                    "meta" => FileKind::DeviceMeta,
                    "icons" => FileKind::AppIcons,
                    "app_groups" => FileKind::AppGroups,
                    "app_group_members" => FileKind::AppGroupMembers,
                    "tombstone" => FileKind::Tombstone,
                    "ai_summaries" => FileKind::AiSummaries,
                    "chat" => FileKind::Chat,
                    _ => return None,
                };
                (id, kind)
            }
            _ => return None,
        };
        Some(Self {
            device_id: device_id.to_string(),
            kind,
        })
    }

    /// Return prefix name of a file, e.g., `device.<device_id>`.
    pub(crate) fn device_prefix(device_id: &str) -> String {
        format!("device.{device_id}.")
    }
}

/// The date in a day file's name, exactly as [`FileName::to_file_name`] writes
/// it: `YYYY-MM-DD`. chrono alone would also take `2026-9-5`, which would not
/// round-trip.
fn parse_date(s: &str) -> Option<NaiveDate> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    (date.to_string() == s).then_some(date)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(kind: FileKind) -> FileName {
        FileName {
            device_id: "abc".into(),
            kind,
        }
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    /// 每一种文件起的名字都认得回来，一个字都不差。
    #[test]
    fn every_kind_survives_a_round_trip() {
        let kinds = [
            FileKind::Activities(date("2026-09-23")),
            FileKind::Categories,
            FileKind::DeviceMeta,
            FileKind::AppIcons,
            FileKind::AppGroups,
            FileKind::AppGroupMembers,
            FileKind::Tombstone,
            FileKind::AiSummaries,
            FileKind::Chat,
            FileKind::Memory(date("2026-09-23")),
        ];
        for kind in kinds {
            let f = file(kind);
            assert_eq!(FileName::parse(&f.to_file_name()), Some(f));
        }
    }

    /// 名字的形状照旧：单文件 `.json`，日文件带日期、`.ndjson`。
    #[test]
    fn names_keep_the_shape_other_versions_read() {
        assert_eq!(
            file(FileKind::Activities(date("2026-09-23"))).to_file_name(),
            "device.abc.activities.2026-09-23.ndjson"
        );
        assert_eq!(
            file(FileKind::Memory(date("2026-09-23"))).to_file_name(),
            "device.abc.memory.2026-09-23.ndjson"
        );
        assert_eq!(
            file(FileKind::DeviceMeta).to_file_name(),
            "device.abc.meta.json"
        );
        assert_eq!(
            file(FileKind::AppIcons).to_file_name(),
            "device.abc.icons.json"
        );
        assert_eq!(
            file(FileKind::AppGroupMembers).to_file_name(),
            "device.abc.app_group_members.json"
        );
        assert_eq!(FileName::device_prefix("abc"), "device.abc.");
    }

    /// 不是同步文件的名字、这个版本不认识的种类、不成日期的日期，都是 `None`。
    #[test]
    fn unknown_names_are_none() {
        for name in [
            "readme.txt",
            "device.abc.notes.json",
            "device.abc.activities.json",
            "device.abc.categories.2026-09-23.ndjson",
            "device.abc.activities.2026-13-45.ndjson",
            "device.abc.activities.2026-9-5.ndjson",
            "manifest.abc.json",
        ] {
            assert_eq!(FileName::parse(name), None, "{name}");
        }
    }

    /// 可选数据集各归自己的流，其余都归核心。
    #[test]
    fn dataset_of_each_kind() {
        assert_eq!(FileKind::AiSummaries.dataset(), Dataset::AiSummaries);
        assert_eq!(FileKind::Chat.dataset(), Dataset::Chat);
        assert_eq!(
            FileKind::Memory(date("2026-09-23")).dataset(),
            Dataset::Memory
        );
        assert_eq!(
            FileKind::Activities(date("2026-09-23")).dataset(),
            Dataset::Core
        );
        assert_eq!(FileKind::Tombstone.dataset(), Dataset::Core);
    }
}
