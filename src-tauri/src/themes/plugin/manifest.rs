//! The theme.json format and its checks. `docs/themes/` lists the fields and rules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The theme.json format this version reads.
const FORMAT: u32 = 1;

/// The size limit of theme.json.
const MAX_JSON_BYTES: usize = 64 * 1024;

/// The size limit of theme.css.
const MAX_CSS_BYTES: usize = 512 * 1024;

/// The length limit of `name` and `author`, in characters.
const MAX_TEXT_CHARS: usize = 64;

/// The length limit of a public variable's value, in characters.
const MAX_TOKEN_VALUE_CHARS: usize = 300;

/// The variables that `tokens` can set. These names are a public contract: renaming one breaks the
/// themes that use it.
const PUBLIC_TOKENS: &[&str] = &[
    // Colors
    "--color-accent",
    "--color-accent-soft",
    "--sidebar-gradient",
    "--sidebar-border",
    "--surface-app",
    "--surface-card",
    "--surface-card-elevated",
    "--surface-overlay",
    "--surface-1",
    "--surface-2",
    "--pill-bg",
    "--pill-blur",
    "--pill-shadow",
    "--hover-bg",
    "--text-strong",
    "--text-regular",
    "--text-muted",
    "--text-faint",
    "--text-on-pill",
    "--status-ok",
    "--status-pending",
    "--status-idle",
    "--status-error",
    "--status-ok-soft",
    "--status-error-soft",
    "--status-pending-soft",
    "--border-subtle",
    "--border-default",
    "--border-strong",
    "--scrollbar-thumb",
    "--scrollbar-thumb-hover",
    "--hairline-hi",
    "--control-bg",
    "--control-bg-hover",
    // Shadows
    "--shadow-sm",
    "--shadow-md",
    "--shadow-lg",
    "--shadow-inset-hi",
    "--shadow-strength",
    // Radii
    "--radius-xs",
    "--radius-sm",
    "--radius-md",
    "--radius-lg",
    "--radius-xl",
    // Fonts
    "--font-sans",
    "--font-mono",
];

/// Whether the page shows its content in cards or flat.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageLayout {
    #[default]
    Card,
    Flat,
}

/// Whether the colors are light or dark.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    Light,
    Dark,
}

/// A checked theme, as sent to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    /// The description in each language: interface language code (such as `zh-CN` or `en`) →
    /// text. The frontend picks one by the interface language
    pub description: BTreeMap<String, String>,
    pub layout: PageLayout,
    pub mode: ColorMode,
    /// The public variables: name → value
    pub tokens: BTreeMap<String, String>,
    /// The variables in theme.json that are not public, sorted by name. They tell the author which
    /// variables had no effect
    pub ignored_tokens: Vec<String>,
    pub has_css: bool,
}

/// Only the format number of theme.json. The rest of the file is not read.
#[derive(Deserialize)]
struct FormatOnly {
    format: u32,
}

/// The contents of theme.json, not yet checked.
#[derive(Deserialize)]
struct ThemeJson {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    description: BTreeMap<String, String>,
    #[serde(default)]
    layout: PageLayout,
    #[serde(default)]
    mode: ColorMode,
    #[serde(default)]
    tokens: BTreeMap<String, String>,
}

/// Parses and checks a theme.
///
/// `json` is the contents of theme.json. `css` is the contents of theme.css, or `None` if there is
/// no theme.css. Variables that are not public are not an error: they go into
/// [`Theme::ignored_tokens`]. Only the size and encoding of theme.css are checked, not its
/// contents.
///
/// # Errors
///
/// Returns [`Error::ThemeInvalid`] if a file is over its size limit, the format is not known, a
/// field or the value of a public variable breaks its rule, or theme.css is not UTF-8.
pub fn parse(json: &[u8], css: Option<&[u8]>) -> Result<Theme> {
    if json.len() > MAX_JSON_BYTES {
        return Err(Error::ThemeInvalid(format!(
            "theme.json is larger than {} KB",
            MAX_JSON_BYTES / 1024
        )));
    }
    // Check the format first: an unknown format must report a format error, not a field error
    // from reading it as format 1
    let FormatOnly { format } = serde_json::from_slice(json).map_err(json_error)?;
    if format != FORMAT {
        return Err(Error::ThemeInvalid(format!(
            "format {format} is not supported; this version reads format {FORMAT}"
        )));
    }
    let file: ThemeJson = serde_json::from_slice(json).map_err(json_error)?;
    if !is_valid_id(&file.id) {
        return Err(Error::ThemeInvalid(format!("invalid id {:?}", file.id)));
    }
    if !is_valid_version(&file.version) {
        return Err(Error::ThemeInvalid(format!(
            "invalid version {:?}",
            file.version
        )));
    }
    if file.name.trim().is_empty() {
        return Err(Error::ThemeInvalid("name cannot be empty".into()));
    }
    for (field, text) in [("name", &file.name), ("author", &file.author)] {
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(Error::ThemeInvalid(format!(
                "{field} is longer than {MAX_TEXT_CHARS} characters"
            )));
        }
    }
    let (tokens, ignored_tokens) = split_tokens(file.tokens)?;
    if let Some(css) = css {
        check_css(css)?;
    }
    Ok(Theme {
        id: file.id,
        name: file.name,
        version: file.version,
        author: file.author,
        description: file.description,
        layout: file.layout,
        mode: file.mode,
        tokens,
        ignored_tokens,
        has_css: css.is_some(),
    })
}

/// Turns a JSON error into a theme error. serde's message has the line and column, so the author
/// can find the mistake.
fn json_error(e: serde_json::Error) -> Error {
    Error::ThemeInvalid(e.to_string())
}

/// Returns whether `id` has only lowercase letters, digits and `-`, does not start or end with `-`,
/// and has at most 63 characters.
///
/// The id becomes a folder name in `themes/`, so it cannot have `/` or `..`. The rule is the same
/// as for plugin ids, but each has its own copy: if plugins change their rule, theme files that are
/// already out should not change with it.
fn is_valid_id(id: &str) -> bool {
    (1..=63).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// Returns whether `version` is `number.number.number`, for example `1.0.0`.
fn is_valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Splits `tokens` into public variables and the rest. The values of public variables are checked.
/// For the rest, only the names are kept.
fn split_tokens(
    tokens: BTreeMap<String, String>,
) -> Result<(BTreeMap<String, String>, Vec<String>)> {
    let mut public = BTreeMap::new();
    let mut ignored = Vec::new();
    for (name, value) in tokens {
        if !PUBLIC_TOKENS.contains(&name.as_str()) {
            ignored.push(name);
            continue;
        }
        if !is_valid_token_value(&value) {
            return Err(Error::ThemeInvalid(format!(
                "{name} must be 1 to {MAX_TOKEN_VALUE_CHARS} characters, \
                 without ;, {{, }} or control characters"
            )));
        }
        public.insert(name, value);
    }
    Ok((public, ignored))
}

/// Returns whether `value` is not empty or only spaces, has at most [`MAX_TOKEN_VALUE_CHARS`]
/// characters, and has no `;`, `{`, `}` or control characters.
///
/// The browser drops a value with `;`, `{` or `}`, and the variable stops working without a sign.
/// Control characters have no use in a value. Reporting them here tells the author at once.
fn is_valid_token_value(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= MAX_TOKEN_VALUE_CHARS
        && !value
            .chars()
            .any(|c| matches!(c, ';' | '{' | '}') || c.is_control())
}

/// Checks that theme.css is at most [`MAX_CSS_BYTES`] and is UTF-8.
fn check_css(css: &[u8]) -> Result<()> {
    if css.len() > MAX_CSS_BYTES {
        return Err(Error::ThemeInvalid(format!(
            "theme.css is larger than {} KB",
            MAX_CSS_BYTES / 1024
        )));
    }
    if std::str::from_utf8(css).is_err() {
        return Err(Error::ThemeInvalid("theme.css is not UTF-8".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个最小的合规 theme.json，再把 `extra` 里的字段放进去（同名的覆盖）。
    fn theme_json(extra: serde_json::Value) -> Vec<u8> {
        let mut json = serde_json::json!({
            "format": 1, "id": "nord", "name": "Nord", "version": "1.0.0"
        });
        let object = json.as_object_mut().unwrap();
        for (key, value) in extra.as_object().unwrap() {
            object.insert(key.clone(), value.clone());
        }
        serde_json::to_vec(&json).unwrap()
    }

    /// 解析应该失败，返回错误里的文字。
    fn error_of(json: &[u8], css: Option<&[u8]>) -> String {
        match parse(json, css) {
            Err(Error::ThemeInvalid(message)) => message,
            other => panic!("应该是 ThemeInvalid，实际是 {other:?}"),
        }
    }

    /// 每个字段都原样读出来；名单里的变量放进 tokens。
    #[test]
    fn parse_reads_every_field() {
        let json = theme_json(serde_json::json!({
            "author": "someone",
            "description": { "en": "Cool blue-grey.", "zh-CN": "冷色调的蓝灰。" },
            "layout": "flat",
            "mode": "dark",
            "tokens": { "--color-accent": "#88c0d0", "--radius-md": "6px" }
        }));

        let theme = parse(&json, None).unwrap();

        assert_eq!(
            theme,
            Theme {
                id: "nord".into(),
                name: "Nord".into(),
                version: "1.0.0".into(),
                author: "someone".into(),
                description: BTreeMap::from([
                    ("en".into(), "Cool blue-grey.".into()),
                    ("zh-CN".into(), "冷色调的蓝灰。".into()),
                ]),
                layout: PageLayout::Flat,
                mode: ColorMode::Dark,
                tokens: BTreeMap::from([
                    ("--color-accent".into(), "#88c0d0".into()),
                    ("--radius-md".into(), "6px".into()),
                ]),
                ignored_tokens: vec![],
                has_css: false,
            }
        );
    }

    /// 只写必填的四个字段：卡片、亮色，其余都是空的。
    #[test]
    fn parse_uses_card_and_light_by_default() {
        let theme = parse(&theme_json(serde_json::json!({})), None).unwrap();

        assert_eq!(theme.layout, PageLayout::Card);
        assert_eq!(theme.mode, ColorMode::Light);
        assert_eq!(theme.author, "");
        assert!(theme.description.is_empty());
        assert!(theme.tokens.is_empty());
        assert!(theme.ignored_tokens.is_empty());
    }

    /// 不认识的格式号：报错里写出这个格式号，不按格式 1 去读。
    #[test]
    fn parse_rejects_an_unknown_format() {
        let message = error_of(&theme_json(serde_json::json!({ "format": 2 })), None);

        assert!(message.contains("format 2"), "{message}");
    }

    /// 没写格式号：报错里写出 format。
    #[test]
    fn parse_rejects_a_missing_format() {
        let json = br#"{ "id": "nord", "name": "Nord", "version": "1.0.0" }"#;

        let message = error_of(json, None);

        assert!(message.contains("format"), "{message}");
    }

    /// 字段不合规则：报错里写出是哪个字段。
    #[test]
    fn parse_rejects_fields_that_break_their_rules() {
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        let cases = [
            (serde_json::json!({ "id": "../evil" }), "id"),
            (serde_json::json!({ "id": "Nord" }), "id"),
            (serde_json::json!({ "version": "1.0" }), "version"),
            (serde_json::json!({ "name": "" }), "name"),
            (serde_json::json!({ "name": "   " }), "name"),
            (serde_json::json!({ "name": long }), "name"),
            (serde_json::json!({ "author": long }), "author"),
            (serde_json::json!({ "layout": "grid" }), "grid"),
            (serde_json::json!({ "mode": "dim" }), "dim"),
        ];

        for (extra, expected) in cases {
            let message = error_of(&theme_json(extra.clone()), None);
            assert!(message.contains(expected), "{extra}: {message}");
        }
    }

    /// 名字和作者正好在长度上限的可以；按字符算，不按字节算。
    #[test]
    fn parse_counts_text_length_in_characters() {
        let name = "蓝".repeat(MAX_TEXT_CHARS);
        let json = theme_json(serde_json::json!({ "name": name, "author": name }));

        assert!(parse(&json, None).is_ok());
    }

    /// 名单里的变量，值不合规则：报错里写出是哪个变量。
    #[test]
    fn parse_rejects_bad_values_of_public_tokens() {
        let long = "x".repeat(MAX_TOKEN_VALUE_CHARS + 1);
        for value in [
            "",
            "  ",
            "red; color: blue",
            "a { b }",
            "red\n",
            long.as_str(),
        ] {
            let json = theme_json(serde_json::json!({ "tokens": { "--color-accent": value } }));

            let message = error_of(&json, None);

            assert!(message.contains("--color-accent"), "{value:?}: {message}");
        }
    }

    /// 值正好在长度上限的可以，带引号、括号、逗号的也可以（字体列表、阴影要用）。
    #[test]
    fn parse_accepts_values_fonts_and_shadows_need() {
        let json = theme_json(serde_json::json!({ "tokens": {
            "--color-accent": "x".repeat(MAX_TOKEN_VALUE_CHARS),
            "--font-sans": "\"IBM Plex Sans\", sans-serif",
            "--shadow-md": "0 1px 2px rgba(0, 0, 0, 0.3), 0 8px 20px rgba(0, 0, 0, 0.35)"
        }}));

        assert_eq!(parse(&json, None).unwrap().tokens.len(), 3);
    }

    /// 名单外的变量不报错，也不检查值：不放进 tokens，列进 ignored_tokens。
    #[test]
    fn parse_moves_tokens_not_in_the_list_to_ignored() {
        let json = theme_json(serde_json::json!({ "tokens": {
            "--color-accent": "#ffffff",
            "--colr-accent": "red",
            "--sidebar-width": "a; b"
        }}));

        let theme = parse(&json, None).unwrap();

        assert_eq!(
            theme.tokens,
            BTreeMap::from([("--color-accent".into(), "#ffffff".into())])
        );
        assert_eq!(theme.ignored_tokens, ["--colr-accent", "--sidebar-width"]);
    }

    /// 不认识的顶层字段忽略：以后的版本加了字段，这个版本照样能读。
    #[test]
    fn parse_ignores_fields_it_does_not_know() {
        let json = theme_json(serde_json::json!({
            "homepage": "https://example.com", "screenshots": ["a.png"]
        }));

        assert!(parse(&json, None).is_ok());
    }

    /// theme.json 超过大小上限：报错里写出上限。
    #[test]
    fn parse_rejects_a_large_theme_json() {
        let padding = "x".repeat(MAX_JSON_BYTES);
        let json = theme_json(serde_json::json!({ "description": { "en": padding } }));

        let message = error_of(&json, None);

        assert!(message.contains("64 KB"), "{message}");
    }

    /// 有 theme.css 时 has_css 为真；正好在大小上限的可以。
    #[test]
    fn parse_accepts_theme_css() {
        let json = theme_json(serde_json::json!({}));
        let css = vec![b' '; MAX_CSS_BYTES];

        assert!(parse(&json, Some(&css)).unwrap().has_css);
    }

    /// theme.css 超过大小上限或不是 UTF-8：报错里写出原因。
    #[test]
    fn parse_rejects_bad_theme_css() {
        let json = theme_json(serde_json::json!({}));
        let large = vec![b' '; MAX_CSS_BYTES + 1];

        let message = error_of(&json, Some(&large));
        assert!(message.contains("512 KB"), "{message}");

        let message = error_of(&json, Some(&[0xff, 0xfe]));
        assert!(message.contains("UTF-8"), "{message}");
    }

    /// 名单里的每个变量，tokens.css 的 :root 里都有定义：名单写错一个字，主题作者设了也没用。
    #[test]
    fn public_tokens_are_defined_in_tokens_css() {
        let css = include_str!("../../../../src/styles/tokens.css");
        let root = css
            .split(":root {")
            .nth(1)
            .and_then(|rest| rest.split("\n}").next())
            .unwrap();

        for token in PUBLIC_TOKENS {
            assert!(root.contains(&format!("{token}:")), "{token}");
        }
    }
}
