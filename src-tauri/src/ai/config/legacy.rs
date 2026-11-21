//! One-time migration from legacy `AiConfig` JSON fields.
//!
//! Moves saved cloud credentials into [`AiConfig::cloud`] and the daily report
//! model into [`AiConfig::picks`]. Chat models, thinking levels, and engine
//! overrides are intentionally not migrated.

use serde::Deserialize;

use super::sanitize::{sanitize_dialect, MAX_CLOUD_CONNECTIONS};
use super::{AiConfig, CloudConnection, ModelRef};

// Legacy `summaryMain` sentinel meaning "use the active cloud connection".
const CLOUD_SENTINEL: &str = "__cloud__";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LegacyAi {
    endpoint: String,
    model: String,
    api_key: String,
    external_enabled: bool,
    external_provider: String,
    external_profiles: Vec<LegacyProfile>,
    active_main: String,
    summary_main: String,
}

impl LegacyAi {
    fn active_profile(&self) -> LegacyProfile {
        LegacyProfile {
            name: String::new(),
            provider: self.external_provider.clone(),
            endpoint: self.endpoint.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LegacyProfile {
    name: String,
    provider: String,
    endpoint: String,
    api_key: String,
    model: String,
}

struct MigratedCloud {
    connections: Vec<CloudConnection>,
    active_id: Option<String>,
}

/// Migrates legacy cloud connections and daily report model selection into `ai`.
///
/// Reads from the raw settings JSON because deserializing [`AiConfig`] drops
/// removed legacy fields. Settings already saved in the new format contain
/// `ai.cloud` and are skipped. Existing `ai.picks.daily` selections are kept.
///
/// Returns `true` if `ai` was modified.
pub fn migrate_legacy_models(raw_settings_json: &str, ai: &mut AiConfig) -> bool {
    let Some(legacy) = parse_legacy_ai(raw_settings_json) else {
        return false;
    };

    let migrated_cloud = collect_cloud_connections(&legacy);
    let mut changed = !migrated_cloud.connections.is_empty();
    ai.cloud.extend(migrated_cloud.connections);

    if ai.picks.daily.is_none() {
        if let Some(daily) = resolve_daily_model(&legacy, migrated_cloud.active_id.as_deref()) {
            ai.picks.daily = Some(daily);
            changed = true;
        }
    }

    changed
}

fn parse_legacy_ai(raw_settings_json: &str) -> Option<LegacyAi> {
    let root: serde_json::Value = serde_json::from_str(raw_settings_json).ok()?;
    let raw_ai = root.get("ai")?;
    // Settings written by the new version always include `ai.cloud` (even when empty).
    if raw_ai.get("cloud").is_some() {
        return None;
    }
    LegacyAi::deserialize(raw_ai).ok()
}

fn collect_cloud_connections(legacy: &LegacyAi) -> MigratedCloud {
    let mut connections = Vec::new();
    let mut active_id = None;

    // Insert the active top-level connection first so `picks.daily` can reference it.
    if let Some(active_conn) = build_connection(&legacy.active_profile(), next_connection_id(0)) {
        active_id = Some(active_conn.id.clone());
        connections.push(active_conn);
    }

    for profile in &legacy.external_profiles {
        if connections.len() >= MAX_CLOUD_CONNECTIONS {
            break;
        }
        let id = next_connection_id(connections.len());
        let Some(conn) = build_connection(profile, id) else {
            continue;
        };

        // The active connection was often saved as a named preset too; merge duplicates
        // and adopt the preset's display name when present.
        if let Some(existing) = connections
            .iter_mut()
            .find(|existing| same_connection(existing, &conn))
        {
            let preset_name = profile.name.trim();
            if !preset_name.is_empty() {
                existing.name = preset_name.to_string();
            }
        } else {
            connections.push(conn);
        }
    }

    MigratedCloud {
        connections,
        active_id,
    }
}

fn resolve_daily_model(legacy: &LegacyAi, active_cloud_id: Option<&str>) -> Option<ModelRef> {
    let summary_main = legacy.summary_main.trim();
    // Legacy startup filled an empty `summaryMain` with `CLOUD_SENTINEL` when cloud was enabled,
    // so both mean "daily uses cloud" while `external_enabled` is true.
    let summary_is_blank_or_sentinel = summary_main.is_empty() || summary_main == CLOUD_SENTINEL;

    if legacy.external_enabled && summary_is_blank_or_sentinel {
        return active_cloud_id.map(|id| ModelRef::Cloud {
            connection: id.to_string(),
        });
    }

    let local_file = if summary_is_blank_or_sentinel {
        legacy.active_main.trim()
    } else {
        summary_main
    };
    (!local_file.is_empty()).then(|| ModelRef::Local {
        file: local_file.to_string(),
    })
}

fn next_connection_id(existing_count: usize) -> String {
    // Legacy profiles have no ID; sequential `c1`, `c2`, ... matches the settings UI.
    format!("c{}", existing_count + 1)
}

fn build_connection(profile: &LegacyProfile, id: String) -> Option<CloudConnection> {
    let endpoint = profile.endpoint.trim().to_string();
    if endpoint.is_empty() {
        return None;
    }

    let model = profile.model.trim().to_string();
    let name = [profile.name.trim(), model.as_str(), endpoint.as_str()]
        .into_iter()
        .find(|candidate| !candidate.is_empty())
        .unwrap_or_default()
        .to_string();

    Some(CloudConnection {
        id,
        name,
        dialect: sanitize_dialect(&profile.provider),
        endpoint,
        model,
        api_key: profile.api_key.trim().to_string(),
        context_length: None,
    })
}

fn same_connection(left: &CloudConnection, right: &CloudConnection) -> bool {
    left.dialect == right.dialect
        && left.endpoint == right.endpoint
        && left.model == right.model
        && left.api_key == right.api_key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::config::{CloudConnection, ModelRef, Pick};

    // Simulate startup load: deserialize into the new AiConfig first (dropping legacy fields),
    // then run migration against the raw JSON.
    fn migrated(ai_json: &str) -> (AiConfig, bool) {
        let data = format!(r#"{{"screenshotPath":"/sp","ai":{ai_json}}}"#);
        let mut ai: AiConfig = serde_json::from_str(ai_json).unwrap();
        let changed = migrate_legacy_models(&data, &mut ai);
        (ai, changed)
    }

    fn cloud(id: &str) -> Option<ModelRef> {
        Some(ModelRef::Cloud {
            connection: id.into(),
        })
    }

    fn local(file: &str) -> Option<ModelRef> {
        Some(ModelRef::Local { file: file.into() })
    }

    #[test]
    fn active_cloud_fields_become_the_first_connection() {
        let (ai, changed) = migrated(
            r#"{
                "externalEnabled": true,
                "externalProvider": " deepseek ",
                "endpoint": " https://api.deepseek.com/v1 ",
                "model": "deepseek-chat",
                "apiKey": "sk-1"
            }"#,
        );
        assert!(changed);
        assert_eq!(
            ai.cloud,
            vec![CloudConnection {
                id: "c1".into(),
                name: "deepseek-chat".into(),
                dialect: "deepseek".into(),
                endpoint: "https://api.deepseek.com/v1".into(),
                model: "deepseek-chat".into(),
                api_key: "sk-1".into(),
                context_length: None,
            }]
        );
    }

    // Matching presets merge into the active connection and adopt the preset's name;
    // blank endpoints are skipped; unknown providers map to `custom`.
    #[test]
    fn saved_profiles_become_connections_without_duplicates() {
        let (ai, _) = migrated(
            r#"{
                "externalProvider": "deepseek",
                "endpoint": "https://api.deepseek.com/v1",
                "model": "deepseek-chat",
                "apiKey": "sk-1",
                "externalProfiles": [
                    {"name": "工作", "provider": "deepseek", "endpoint": "https://api.deepseek.com/v1", "model": "deepseek-chat", "apiKey": "sk-1"},
                    {"name": "", "provider": "openai", "endpoint": "https://api.openai.com/v1", "model": "gpt-4o-mini", "apiKey": "sk-2"},
                    {"name": "空", "provider": "openai", "endpoint": "  ", "model": "x", "apiKey": ""},
                    {"name": "", "provider": "某家", "endpoint": "http://10.0.0.2:8000/v1", "model": "", "apiKey": ""}
                ]
            }"#,
        );
        let got: Vec<(&str, &str, &str, &str)> = ai
            .cloud
            .iter()
            .map(|c| {
                (
                    c.id.as_str(),
                    c.name.as_str(),
                    c.dialect.as_str(),
                    c.api_key.as_str(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("c1", "工作", "deepseek", "sk-1"),
                ("c2", "gpt-4o-mini", "openai", "sk-2"),
                ("c3", "http://10.0.0.2:8000/v1", "custom", ""),
            ]
        );
    }

    #[test]
    fn at_most_ten_connections_are_migrated() {
        let profiles: Vec<String> = (0..12)
            .map(|i| {
                format!(
                    r#"{{"name":"p{i}","provider":"openai","endpoint":"https://e{i}/v1","model":"m","apiKey":""}}"#
                )
            })
            .collect();
        let (ai, _) = migrated(&format!(
            r#"{{"externalProfiles":[{}]}}"#,
            profiles.join(",")
        ));
        assert_eq!(ai.cloud.len(), 10);
        assert_eq!(ai.cloud[9].name, "p9");
    }

    // When `externalEnabled` is false, a stale `__cloud__` sentinel falls back to `activeMain`.
    #[test]
    fn daily_uses_the_cloud_when_it_did_before() {
        let fields = r#""endpoint":"https://api.x/v1","model":"m","activeMain":"a.gguf""#;
        for (extra, want) in [
            (
                r#""externalEnabled":true,"summaryMain":"__cloud__""#,
                cloud("c1"),
            ),
            (r#""externalEnabled":true,"summaryMain":"  ""#, cloud("c1")),
            (
                r#""externalEnabled":false,"summaryMain":"__cloud__""#,
                local("a.gguf"),
            ),
        ] {
            let (ai, changed) = migrated(&format!("{{{fields},{extra}}}"));
            assert!(changed, "{extra}");
            assert_eq!(ai.picks.daily, want, "{extra}");
        }
    }

    #[test]
    fn daily_stays_unset_when_the_old_cloud_had_no_endpoint() {
        let (ai, changed) =
            migrated(r#"{"externalEnabled":true,"summaryMain":"","activeMain":"a.gguf"}"#);
        assert!(!changed);
        assert!(ai.cloud.is_empty());
        assert_eq!(ai.picks.daily, None);
    }

    #[test]
    fn daily_local_prefers_summary_main_over_active_main() {
        for (json, want) in [
            (
                r#"{"summaryMain":"s.gguf","activeMain":"a.gguf"}"#,
                local("s.gguf"),
            ),
            (
                r#"{"summaryMain":"","activeMain":" a.gguf "}"#,
                local("a.gguf"),
            ),
            (r#"{"summaryMain":"","activeMain":""}"#, None),
        ] {
            let (ai, _) = migrated(json);
            assert_eq!(ai.picks.daily, want, "{json}");
        }
    }

    // Chat model, thinking level, and engine overrides are intentionally not migrated.
    #[test]
    fn only_connections_and_daily_model_are_migrated() {
        let (ai, _) = migrated(
            r#"{
                "endpoint": "https://api.x/v1", "model": "m",
                "chatMain": "chat.gguf", "chatThinking": "high",
                "ctxSize": 32768, "summaryCtxSize": 16384,
                "activeMain": "a.gguf",
                "picks": {"weekly": {"kind": "local", "file": "w.gguf"}}
            }"#,
        );
        assert_eq!(ai.picks.daily, local("a.gguf"));
        assert_eq!(ai.picks.chat, None);
        assert_eq!(
            ai.picks.weekly,
            Pick::Local {
                file: "w.gguf".into()
            }
        );
        assert!(ai.advanced.is_empty());
    }

    // Presence of `cloud` (even empty) marks a new-format config so deleted connections are not
    // re-added from leftover legacy fields.
    #[test]
    fn settings_saved_by_the_new_version_are_left_alone() {
        let (ai, changed) = migrated(
            r#"{"cloud": [], "picks": {}, "endpoint": "https://api.x/v1", "model": "m", "activeMain": "a.gguf"}"#,
        );
        assert!(!changed);
        assert!(ai.cloud.is_empty());
        assert_eq!(ai.picks.daily, None);
    }

    #[test]
    fn an_existing_daily_pick_is_kept() {
        let data = r#"{"ai":{"activeMain":"a.gguf","endpoint":"https://api.x/v1","model":"m"}}"#;
        let mut ai = AiConfig {
            picks: crate::ai::config::Picks {
                daily: local("new.gguf"),
                ..Default::default()
            },
            ..AiConfig::default()
        };
        migrate_legacy_models(data, &mut ai);
        assert_eq!(ai.picks.daily, local("new.gguf"));
        assert_eq!(ai.cloud.len(), 1);
    }
}
