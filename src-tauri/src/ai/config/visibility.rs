//! AI 能看到哪些分类：日报、周报、对话各存一组配置。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The profiles of the daily report, the weekly report and chat.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Visibility {
    pub daily: ModuleProfiles,
    pub weekly: ModuleProfiles,
    pub chat: ModuleProfiles,
}

/// The profiles of one feature.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ModuleProfiles {
    /// Profile id → profile. Ids are generated, so renaming a profile does not break
    /// references to it.
    pub profiles: BTreeMap<String, VisibilityProfile>,
    /// The profile that automatic generation uses. None means the main settings.
    pub auto_profile: Option<String>,
}

/// A profile: which categories the AI can see. The AI cannot see the categories that are
/// not selected.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct VisibilityProfile {
    pub name: String,
    /// Ids of the selected categories.
    pub categories: Vec<String>,
}
