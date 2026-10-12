use std::time::Duration;

/// GitHub Releases endpoint, `<repo>/releases/tags/<tag>`. Its `body` is the full text of
/// `docs/release-notes/<tag>.md` as attached at release time, including every language block.
const RELEASE_API: &str = "https://api.github.com/repos/Tomotsugu-dev/Hindsight/releases/tags";

/// Only a fallback for when the frontend bundle lacks the version's notes, so give up quickly
/// instead of leaving the user watching an empty dialog.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Fetches the GitHub Release notes for `version` ("0.9.3" or "v0.9.3") for the About page.
///
/// This runs in the backend because the CSP `connect-src` does not allow external hosts and
/// the timeouts are set here. It is called only when the user opens the release notes and the
/// bundled copy is missing; nothing polls it.
///
/// # Errors
/// Returns a message when the version is malformed, the request fails or times out, GitHub
/// answers with an error status (for example 404 when the tag has no Release), or the Release
/// body is empty.
#[tauri::command]
pub async fn fetch_release_notes(version: String) -> Result<String, String> {
    let tag = release_tag(&version)?;
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        // The GitHub API rejects requests without a User-Agent.
        .user_agent(concat!("Hindsight/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("{RELEASE_API}/{tag}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    release_body(&json)
}

/// "0.9.3" or "v0.9.3" becomes "v0.9.3". The version is spliced into a URL, so only the
/// characters a semver string can contain are accepted.
fn release_tag(version: &str) -> Result<String, String> {
    let v = version.trim().trim_start_matches('v');
    let ok = !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    if ok {
        Ok(format!("v{v}"))
    } else {
        Err(format!("invalid version: {version}"))
    }
}

/// Returns the Release `body`. An empty body counts as a failure so the caller shows its
/// "no release notes" text rather than a blank dialog.
fn release_body(json: &serde_json::Value) -> Result<String, String> {
    match json.get("body").and_then(|b| b.as_str()) {
        Some(b) if !b.trim().is_empty() => Ok(b.to_string()),
        _ => Err("release has no body".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tag_accepts_with_or_without_v() {
        assert_eq!(release_tag("0.9.3").unwrap(), "v0.9.3");
        assert_eq!(release_tag("v0.9.3").unwrap(), "v0.9.3");
        assert_eq!(release_tag("1.0.0-beta.1").unwrap(), "v1.0.0-beta.1");
    }

    #[test]
    fn tag_rejects_anything_that_could_alter_the_url() {
        assert!(release_tag("").is_err());
        assert!(release_tag("../x").is_err());
        assert!(release_tag("0.9.3/../../x").is_err());
        assert!(release_tag("0.9.3?x=1").is_err());
    }

    #[test]
    fn body_must_be_non_empty_string() {
        assert_eq!(release_body(&json!({"body": "hi"})).unwrap(), "hi");
        assert!(release_body(&json!({"body": "  "})).is_err());
        assert!(release_body(&json!({"body": null})).is_err());
        assert!(release_body(&json!({})).is_err());
    }
}
