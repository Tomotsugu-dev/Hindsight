//! Which categories the AI can see. There is one set of profiles, and daily reports,
//! weekly reports and chat all pick from it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// All the profiles, and which one daily and weekly reports each use when they run
/// automatically.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Visibility {
    /// Profile id → profile. Ids are generated, so renaming a profile does not break
    /// references to it.
    pub profiles: BTreeMap<String, VisibilityProfile>,
    pub auto: AutoProfiles,
}

/// Ids of the profiles that automatic runs use. None means the default categories.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoProfiles {
    pub daily: Option<String>,
    pub weekly: Option<String>,
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
