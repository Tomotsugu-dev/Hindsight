//! What goes on the island: reads the JSON it is given into [`Content`] and checks the rules. For
//! the rules and the reasons, see island.md in docs/design/plugins/.
//!
//! All widths are display widths: a full-width character is 2, any other character 1.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::error::Error;

/// Text on each side of the closed island
const CLOSED_TEXT_WIDTH: usize = 6;
/// Text in the middle of a ring
const RING_TEXT_WIDTH: usize = 6;
/// Large text
const LARGE_TEXT_WIDTH: usize = 8;
/// The `sub` after large text
const LARGE_SUB_WIDTH: usize = 14;
/// Small text and its `sub` together; the text on a button
const SMALL_TEXT_WIDTH: usize = 20;
/// The most components on the left of the open island
const MAX_OPEN_LEFT: usize = 3;
/// The most components on the right of the open island
const MAX_OPEN_RIGHT: usize = 2;
/// The most cells in a row
const MAX_COLUMNS: usize = 20;
/// The most rows of cells
const MAX_ROWS: usize = 3;
/// The most bytes in an icon name or a button `id`
const MAX_NAME_BYTES: usize = 64;

/// Everything on the island. All of it has been checked, and it has no unknown components.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Content {
    pub closed: ClosedContent,
    /// `None`: the island does not open when the mouse is over it
    pub open: Option<OpenContent>,
}

/// What is on each side of the closed island. At least one side has something.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClosedContent {
    pub left: Option<Component>,
    pub right: Option<Component>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenContent {
    pub top_right: Option<Text>,
    pub left: Vec<Component>,
    pub right: Vec<Component>,
    /// 右边那一行下面，靠右
    pub bottom_right: Option<Text>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", expecting = "a component")]
pub enum Component {
    Icon(Icon),
    Text(Text),
    Timer(Timer),
    Ring(Ring),
    Cells(Cells),
    Button(Button),
    /// A component this version does not know. Skipped when reading, so never kept in [`Content`]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Icon {
    /// The name of a category icon in the main window. Not checked: for a name it does not know,
    /// the page shows a default icon
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Text {
    /// Can be given as a single string, but is always read into a list of runs
    #[serde(deserialize_with = "runs")]
    pub text: Vec<Run>,
    /// Small grey text after it
    pub sub: Option<String>,
    #[serde(default)]
    pub size: Size,
}

/// A piece of text in one style.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    /// The page makes it lighter when it is too dark
    pub color: Option<String>,
    #[serde(default)]
    pub dim: bool,
    #[serde(default)]
    pub bold: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    #[default]
    Small,
    Large,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timer {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    /// When set, the timer stops at the time left at this moment
    pub paused_at: Option<DateTime<FixedOffset>>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ring {
    /// From 0 to 1
    pub value: f64,
    /// In the middle of the ring
    pub text: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cells {
    /// One color for each cell; `None` is an empty cell
    pub colors: Vec<Option<String>>,
    pub columns: usize,
    /// The index of the current cell
    pub current: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Button {
    /// Handed back unchanged to whoever put the button there, when it is clicked
    pub id: String,
    pub icon: String,
    pub text: Option<String>,
}

/// Only the outer part is read first, and each component is kept as JSON and read on its own: this
/// way an error can say where it is
#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct Raw {
    closed: RawClosed,
    open: Option<RawOpen>,
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct RawClosed {
    left: Option<Value>,
    right: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct RawOpen {
    top_right: Option<Value>,
    #[serde(default)]
    left: Vec<Value>,
    #[serde(default)]
    right: Vec<Value>,
    bottom_right: Option<Value>,
}

/// Where a component is: the rules differ
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Closed,
    Open,
}

impl Content {
    /// Reads the content it is given and checks the rules.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInputDyn`] when the format is wrong or a rule is broken. The message starts
    /// with the place, such as `open.left[1]: ...`.
    pub fn parse(value: Value) -> Result<Self, Error> {
        let raw: Raw =
            serde_json::from_value(value).map_err(|e| Error::InvalidInputDyn(e.to_string()))?;
        Ok(Content {
            closed: closed(raw.closed)?,
            open: raw.open.map(open).transpose()?.flatten(),
        })
    }

    /// Returns whether the open island has a button.
    pub fn has_button(&self) -> bool {
        self.open.as_ref().is_some_and(|open| {
            open.left
                .iter()
                .chain(&open.right)
                .any(|component| matches!(component, Component::Button(_)))
        })
    }
}

fn closed(raw: RawClosed) -> Result<ClosedContent, Error> {
    if raw.left.is_none() && raw.right.is_none() {
        return Err(invalid("closed", "needs left or right"));
    }
    let side = |value: Option<Value>, path: &str| match value {
        Some(value) => component(value, path, Place::Closed),
        None => Ok(None),
    };
    let left = side(raw.left, "closed.left")?;
    let right = side(raw.right, "closed.right")?;
    // Both sides are unknown components: the island would be empty, and the mouse could neither
    // go over it nor click it
    if left.is_none() && right.is_none() {
        return Err(invalid(
            "closed",
            "nothing in it can be shown by this version of Hindsight",
        ));
    }
    Ok(ClosedContent { left, right })
}

/// Returns `None` when there is nothing in it, the same as when it is not given.
fn open(raw: RawOpen) -> Result<Option<OpenContent>, Error> {
    // Counts what was given, unknown components too: the same content gives the same result on
    // old and new versions
    for (path, list, max) in [
        ("open.left", &raw.left, MAX_OPEN_LEFT),
        ("open.right", &raw.right, MAX_OPEN_RIGHT),
    ] {
        if list.len() > max {
            return Err(invalid(
                path,
                format!("has {} components, at most {max}", list.len()),
            ));
        }
    }
    let top_right = small_text(raw.top_right, "open.topRight")?;
    let list = |values: Vec<Value>, side: &str| -> Result<Vec<Component>, Error> {
        let mut components = Vec::new();
        for (i, value) in values.into_iter().enumerate() {
            components.extend(component(value, &format!("open.{side}[{i}]"), Place::Open)?);
        }
        Ok(components)
    };
    let left = list(raw.left, "left")?;
    let right = list(raw.right, "right")?;
    let bottom_right = small_text(raw.bottom_right, "open.bottomRight")?;
    if top_right.is_none() && left.is_empty() && right.is_empty() && bottom_right.is_none() {
        return Ok(None);
    }
    Ok(Some(OpenContent {
        top_right,
        left,
        right,
        bottom_right,
    }))
}

/// 读只能放一个小字的地方：`open.topRight`、`open.bottomRight`。不认识的组件返回 `None`。
fn small_text(value: Option<Value>, path: &str) -> Result<Option<Text>, Error> {
    let Some(value) = value else {
        return Ok(None);
    };
    match component(value, path, Place::Open)? {
        Some(Component::Text(text)) if text.size == Size::Small => Ok(Some(text)),
        Some(_) => Err(invalid(path, "only small text can go here")),
        None => Ok(None),
    }
}

/// Reads one component and checks it for where it is. Returns `None` for an unknown component.
fn component(value: Value, path: &str, place: Place) -> Result<Option<Component>, Error> {
    let component: Component = serde_json::from_value(value).map_err(|e| invalid(path, e))?;
    check(&component, place).map_err(|e| invalid(path, e))?;
    Ok(match component {
        Component::Unknown => None,
        component => Some(component),
    })
}

fn check(component: &Component, place: Place) -> Result<(), String> {
    match component {
        Component::Icon(icon) => {
            name("name", &icon.name)?;
            color(icon.color.as_deref())
        }
        Component::Text(text) => check_text(text, place),
        Component::Timer(timer) => {
            if timer.start >= timer.end {
                return Err("start must be before end".into());
            }
            if timer
                .paused_at
                .is_some_and(|at| at < timer.start || at > timer.end)
            {
                return Err("pausedAt must be between start and end".into());
            }
            color(timer.color.as_deref())
        }
        Component::Ring(ring) => {
            if !(0.0..=1.0).contains(&ring.value) {
                return Err(format!("value must be 0 to 1, not {}", ring.value));
            }
            if let Some(text) = &ring.text {
                string("text", text)?;
                at_most("text", width(text), RING_TEXT_WIDTH)?;
            }
            color(ring.color.as_deref())
        }
        Component::Cells(cells) => {
            if place == Place::Closed {
                return Err("cells can only go in open".into());
            }
            if !(1..=MAX_COLUMNS).contains(&cells.columns) {
                return Err(format!(
                    "columns must be 1 to {MAX_COLUMNS}, not {}",
                    cells.columns
                ));
            }
            let count = cells.colors.len();
            if count == 0 {
                return Err("colors is empty".into());
            }
            let rows = count.div_ceil(cells.columns);
            if rows > MAX_ROWS {
                return Err(format!(
                    "{count} cells in {} columns take {rows} rows, at most {MAX_ROWS}",
                    cells.columns
                ));
            }
            if cells.current.is_some_and(|i| i >= count) {
                return Err(format!("current must be below {count}"));
            }
            cells.colors.iter().try_for_each(|c| color(c.as_deref()))
        }
        Component::Button(button) => {
            if place == Place::Closed {
                return Err("button can only go in open".into());
            }
            name("id", &button.id)?;
            name("icon", &button.icon)?;
            match &button.text {
                Some(text) => {
                    string("text", text)?;
                    at_most("text", width(text), SMALL_TEXT_WIDTH)
                }
                None => Ok(()),
            }
        }
        Component::Unknown => Ok(()),
    }
}

fn check_text(text: &Text, place: Place) -> Result<(), String> {
    if text.text.is_empty() {
        return Err("text has no runs".into());
    }
    for run in &text.text {
        string("text", &run.text)?;
        color(run.color.as_deref())?;
    }
    if let Some(sub) = &text.sub {
        string("sub", sub)?;
    }
    let text_width = text.text.iter().map(|run| width(&run.text)).sum();
    let sub_width = text.sub.as_deref().map_or(0, width);
    match (place, text.size) {
        // The closed island is only as tall as the menu bar: no room for large text or for a
        // second piece of small text
        (Place::Closed, Size::Large) => Err("text in closed must be small".into()),
        (Place::Closed, Size::Small) if text.sub.is_some() => {
            Err("text in closed cannot have sub".into())
        }
        (Place::Closed, Size::Small) => at_most("text", text_width, CLOSED_TEXT_WIDTH),
        // Small text and its sub are on one line, in one size
        (Place::Open, Size::Small) => {
            at_most("text + sub", text_width + sub_width, SMALL_TEXT_WIDTH)
        }
        (Place::Open, Size::Large) => {
            at_most("text", text_width, LARGE_TEXT_WIDTH)?;
            at_most("sub", sub_width, LARGE_SUB_WIDTH)
        }
    }
}

/// Reads `text`: a string, or a list of runs, where a run can also be just a string.
fn runs<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Run>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged, expecting = "a string, or a list of strings and runs")]
    enum Input {
        One(String),
        Many(Vec<RunInput>),
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RunInput {
        Plain(String),
        Styled(Run),
    }
    let plain = |text| Run {
        text,
        ..Run::default()
    };
    Ok(match Input::deserialize(deserializer)? {
        Input::One(text) => vec![plain(text)],
        Input::Many(runs) => runs
            .into_iter()
            .map(|run| match run {
                RunInput::Plain(text) => plain(text),
                RunInput::Styled(run) => run,
            })
            .collect(),
    })
}

/// Checks that it is not empty and has no line breaks or other control characters: text on the
/// island is one line.
fn string(field: &str, s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err(format!("{field} is empty"));
    }
    // U+2028 and U+2029 break lines but are not control characters
    if s.chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(format!("{field} has a line break or control character"));
    }
    Ok(())
}

fn name(field: &str, s: &str) -> Result<(), String> {
    string(field, s)?;
    if s.len() > MAX_NAME_BYTES {
        return Err(format!(
            "{field} is {} bytes, at most {MAX_NAME_BYTES}",
            s.len()
        ));
    }
    Ok(())
}

fn color(color: Option<&str>) -> Result<(), String> {
    let is_rrggbb = |c: &str| {
        c.strip_prefix('#')
            .is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    match color {
        Some(c) if !is_rrggbb(c) => Err("color must be #rrggbb".into()),
        _ => Ok(()),
    }
}

fn at_most(field: &str, width: usize, max: usize) -> Result<(), String> {
    if width > max {
        return Err(format!("{field} is {width} wide, at most {max}"));
    }
    Ok(())
}

/// Returns the display width. Combining marks and the joiners inside emoji count 1 too: counting
/// too much only makes the limits stricter, and never lets text run out of the island.
fn width(s: &str) -> usize {
    s.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum()
}

fn is_wide(c: char) -> bool {
    matches!(
        c as u32,
        0x1100..=0x115F // Hangul letters
            | 0x231A..=0x231B | 0x23E9..=0x23F3 | 0x2600..=0x27BF // Symbols shown as emoji
            | 0x2B1B..=0x2B1C | 0x2B50 | 0x2B55
            | 0x2E80..=0x303E // CJK radicals and punctuation
            | 0x3041..=0x33FF // Kana, Bopomofo, Hangul compatibility letters, CJK symbols
            | 0x3400..=0x4DBF // CJK Extension A
            | 0x4E00..=0x9FFF // CJK ideographs
            | 0xA000..=0xA4CF // Yi
            | 0xAC00..=0xD7A3 // Hangul syllables
            | 0xF900..=0xFAFF // CJK compatibility ideographs
            | 0xFE30..=0xFE4F // CJK compatibility forms
            | 0xFF00..=0xFF60 // Full-width letters and punctuation
            | 0xFFE0..=0xFFE6 // Full-width signs
            | 0x1F000..=0x1FAFF // Emoji, mahjong tiles, playing cards, flags
            | 0x20000..=0x3FFFD // CJK Extension B and later
    )
}

fn invalid(path: &str, message: impl std::fmt::Display) -> Error {
    Error::InvalidInputDyn(format!("{path}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn icon() -> Value {
        json!({ "type": "icon", "name": "Code" })
    }

    /// 收起时左边放一个图标，展开后放 `open`
    fn with_open(open: Value) -> Value {
        json!({ "closed": { "left": icon() }, "open": open })
    }

    /// 展开后左边放一个组件
    fn in_open(component: Value) -> Value {
        with_open(json!({ "left": [component] }))
    }

    /// 收起时左边放一个组件
    fn in_closed(component: Value) -> Value {
        json!({ "closed": { "left": component } })
    }

    fn error(value: Value) -> String {
        match Content::parse(value) {
            Err(Error::InvalidInputDyn(message)) => message,
            other => panic!("应该报错，结果是 {other:?}"),
        }
    }

    fn run(text: &str) -> Run {
        Run {
            text: text.into(),
            ..Run::default()
        }
    }

    #[test]
    fn reads_the_focus_example() {
        // island.md 里的例子：专注插件开始一段 25 分钟的专注
        let timer = json!({
            "type": "timer",
            "start": "2026-10-21T10:00:00+09:00",
            "end": "2026-10-21T10:25:00+09:00",
            "color": "#fb923c"
        });
        let content = Content::parse(json!({
            "closed": {
                "left": { "type": "icon", "name": "Code", "color": "#a78bfa" },
                "right": timer
            },
            "open": {
                "topRight": {
                    "type": "text",
                    "text": [{ "text": "工作中", "bold": true }, { "text": " · Code", "dim": true }]
                },
                "left": [
                    {
                        "type": "text",
                        "text": [{ "text": "2:40", "bold": true }],
                        "sub": "/ 6:00",
                        "size": "large"
                    },
                    {
                        "type": "cells",
                        "colors": ["#a78bfa", "#a78bfa", "#f87171", null, null],
                        "current": 2,
                        "columns": 15
                    }
                ],
                "right": [timer, { "type": "button", "id": "pause", "icon": "Pause" }],
                "bottomRight": {
                    "type": "text",
                    "text": ["分心 ", { "text": "3", "color": "#f87171", "bold": true }, " 次"]
                }
            }
        }))
        .unwrap();
        assert!(content.has_button());
        let open = content.open.unwrap();
        assert_eq!(
            open.left[0],
            Component::Text(Text {
                text: vec![Run {
                    text: "2:40".into(),
                    bold: true,
                    ..Run::default()
                }],
                sub: Some("/ 6:00".into()),
                size: Size::Large,
            })
        );
        assert_eq!(open.right.len(), 2);
        // 字符串段是不带样式的一段
        assert_eq!(
            open.bottom_right.unwrap().text,
            vec![
                run("分心 "),
                Run {
                    text: "3".into(),
                    color: Some("#f87171".into()),
                    bold: true,
                    ..Run::default()
                },
                run(" 次"),
            ]
        );
        assert_eq!(
            open.top_right.unwrap().text[1],
            Run {
                text: " · Code".into(),
                dim: true,
                ..Run::default()
            }
        );
    }

    #[test]
    fn the_page_always_gets_a_list_of_runs() {
        // 页面拿到的格式：只给一个字符串的 text 也变成一列段，没传的字段都在
        let content = Content::parse(in_closed(json!({ "type": "text", "text": "专注" }))).unwrap();
        assert_eq!(
            serde_json::to_value(&content.closed.left).unwrap(),
            json!({
                "type": "text",
                "text": [{ "text": "专注", "color": null, "dim": false, "bold": false }],
                "sub": null,
                "size": "small"
            })
        );
    }

    #[test]
    fn unknown_components_and_fields_are_skipped() {
        // 以后才有的组件、已有组件上以后加的字段：这个版本跳过，不报错
        let content = Content::parse(with_open(json!({
            "left": [{ "type": "chart" }, { "type": "icon", "name": "Code", "blink": true }]
        })))
        .unwrap();
        assert_eq!(
            content.open.unwrap().left,
            vec![Component::Icon(Icon {
                name: "Code".into(),
                color: None,
            })]
        );
    }

    #[test]
    fn closed_needs_something_to_show() {
        assert_eq!(
            error(json!({ "closed": {} })),
            "closed: needs left or right"
        );
        // 两边都不认识：岛上什么都没有，鼠标移不上去
        assert_eq!(
            error(json!({ "closed": { "left": { "type": "chart" }, "right": { "type": "map" } } })),
            "closed: nothing in it can be shown by this version of Hindsight"
        );
        // 只有一边不认识：照常显示另一边
        let content =
            Content::parse(json!({ "closed": { "left": { "type": "chart" }, "right": icon() } }))
                .unwrap();
        assert_eq!(content.closed.left, None);
        assert!(content.closed.right.is_some());
    }

    #[test]
    fn open_can_be_left_out() {
        let content = Content::parse(in_closed(icon())).unwrap();
        assert_eq!(content.open, None);
        assert!(!content.has_button());
        // 里面全是不认识的组件，和不传一样
        let content = Content::parse(in_open(json!({ "type": "chart" }))).unwrap();
        assert_eq!(content.open, None);
    }

    #[test]
    fn some_components_only_go_in_open() {
        for component in [
            json!({ "type": "cells", "colors": [null], "columns": 1 }),
            json!({ "type": "button", "id": "pause", "icon": "Pause" }),
            json!({ "type": "text", "text": "2:40", "size": "large" }),
            json!({ "type": "text", "text": "2:40", "sub": "/ 4:00" }),
        ] {
            assert!(error(in_closed(component.clone())).starts_with("closed.left: "));
            assert!(Content::parse(in_open(component)).is_ok());
        }
    }

    #[test]
    fn the_top_right_and_the_bottom_right_take_only_small_text() {
        for place in ["topRight", "bottomRight"] {
            for component in [
                icon(),
                json!({ "type": "text", "text": "2:40", "size": "large" }),
            ] {
                assert_eq!(
                    error(with_open(json!({ place: component }))),
                    format!("open.{place}: only small text can go here")
                );
            }
            // 只放了这一处，展开时也有东西
            let content = Content::parse(with_open(
                json!({ place: { "type": "text", "text": "分心 3 次" } }),
            ))
            .unwrap();
            assert!(content.open.is_some());
            // 不认识的组件跳过
            let content = Content::parse(with_open(json!({ place: { "type": "chart" } }))).unwrap();
            assert_eq!(content.open, None);
        }
    }

    #[test]
    fn open_holds_three_on_the_left_and_two_on_the_right() {
        let icon = icon();
        let full = json!({ "left": [icon, icon, icon], "right": [icon, icon] });
        assert!(Content::parse(with_open(full)).is_ok());
        // 不认识的组件也算个数
        assert_eq!(
            error(with_open(
                json!({ "left": [icon, icon, icon, { "type": "chart" }] })
            )),
            "open.left: has 4 components, at most 3"
        );
        assert_eq!(
            error(with_open(json!({ "right": [icon, icon, icon] }))),
            "open.right: has 3 components, at most 2"
        );
    }

    #[test]
    fn errors_say_where() {
        // 下标按交来的算，跳过的组件也占一个
        assert_eq!(
            error(with_open(
                json!({ "left": [{ "type": "chart" }, { "type": "icon", "name": "" }] })
            )),
            "open.left[1]: name is empty"
        );
        assert!(error(in_open(
            json!({ "type": "text", "text": "a", "size": "huge" })
        ))
        .starts_with("open.left[0]: unknown variant `huge`"));
        assert!(error(in_open(json!({ "type": "text", "text": 3 })))
            .contains("a string, or a list of strings and runs"));
        assert_eq!(error(json!({})), "missing field `closed`");
        // 不是对象时，消息里不出现 Rust 的类型名
        assert!(error(json!({ "closed": "icon" })).ends_with("expected an object"));
        assert_eq!(
            error(in_closed(json!("icon"))),
            r#"closed.left: invalid type: string "icon", expected a component"#
        );
    }

    #[test]
    fn full_width_characters_count_two() {
        let closed =
            |text: Value| Content::parse(in_closed(json!({ "type": "text", "text": text })));
        // 收起时的字最多 6：半角 6 个，或全角 3 个
        for ok in ["abcdef", "专注中", "ｱｲｳｴｵｶ", "😀😀😀", "⏰⭐⌛"] {
            assert!(closed(json!(ok)).is_ok(), "{ok}");
        }
        // ⏰ ⭐ ⌛ 不在 emoji 的那一大段里，也显示成 emoji，要算 2
        for bad in ["abcdefg", "专注中a", "😀😀😀a", "⏰⭐⌛a"] {
            assert!(closed(json!(bad)).is_err(), "{bad}");
        }
        assert_eq!(
            error(in_closed(json!({ "type": "text", "text": "专注中了" }))),
            "closed.left: text is 8 wide, at most 6"
        );
        // 几段合起来算
        assert!(closed(json!(["专注", { "text": "中了", "bold": true }])).is_err());
    }

    #[test]
    fn small_text_and_its_sub_share_twenty() {
        let text =
            |text: &str, sub: &str| in_open(json!({ "type": "text", "text": text, "sub": sub }));
        assert!(Content::parse(text("1234567890", "一二三四五")).is_ok());
        assert_eq!(
            error(text("1234567890", "一二三四五a")),
            "open.left[0]: text + sub is 21 wide, at most 20"
        );
        // 按钮上的字也是小字
        let button = |text: &str| {
            in_open(json!({ "type": "button", "id": "a", "icon": "Pause", "text": text }))
        };
        assert!(Content::parse(button("一二三四五六七八九十")).is_ok());
        assert!(Content::parse(button("一二三四五六七八九十a")).is_err());
    }

    #[test]
    fn large_text_takes_eight_and_its_sub_fourteen() {
        let large = |text: &str, sub: &str| {
            in_open(json!({ "type": "text", "text": text, "sub": sub, "size": "large" }))
        };
        assert!(Content::parse(large("12345678", "12345678901234")).is_ok());
        assert_eq!(
            error(large("123456789", "a")),
            "open.left[0]: text is 9 wide, at most 8"
        );
        assert_eq!(
            error(large("1", "123456789012345")),
            "open.left[0]: sub is 15 wide, at most 14"
        );
    }

    #[test]
    fn colors_are_rrggbb() {
        let icon =
            |color: &str| in_closed(json!({ "type": "icon", "name": "Code", "color": color }));
        for ok in ["#a78bfa", "#A78BFA"] {
            assert!(Content::parse(icon(ok)).is_ok(), "{ok}");
        }
        for bad in ["red", "#fff", "#a78bfa80", "#gggggg", "a78bfa"] {
            assert_eq!(
                error(icon(bad)),
                "closed.left: color must be #rrggbb",
                "{bad}"
            );
        }
        // 段和格子里的颜色也查
        assert!(Content::parse(in_closed(
            json!({ "type": "text", "text": [{ "text": "a", "color": "red" }] })
        ))
        .is_err());
        assert!(Content::parse(in_open(
            json!({ "type": "cells", "colors": ["red"], "columns": 1 })
        ))
        .is_err());
    }

    #[test]
    fn a_timer_ends_after_it_starts() {
        let timer = |end: &str, paused_at: Option<&str>| {
            in_closed(json!({
                "type": "timer",
                "start": "2026-10-21T10:00:00+09:00",
                "end": end,
                "pausedAt": paused_at
            }))
        };
        let end = "2026-10-21T10:25:00+09:00";
        // 停在开头、停在结尾都行
        for paused_at in [None, Some("2026-10-21T10:00:00+09:00"), Some(end)] {
            assert!(
                Content::parse(timer(end, paused_at)).is_ok(),
                "{paused_at:?}"
            );
        }
        // 时区不同也按同一时刻比：01:00Z 就是 10:00+09:00
        assert_eq!(
            error(timer("2026-10-21T01:00:00Z", None)),
            "closed.left: start must be before end"
        );
        for paused_at in ["2026-10-21T09:59:59+09:00", "2026-10-21T10:25:01+09:00"] {
            assert_eq!(
                error(timer(end, Some(paused_at))),
                "closed.left: pausedAt must be between start and end"
            );
        }
        // 没有时区的时间读不了
        assert!(Content::parse(timer("2026-10-21T10:25:00", None)).is_err());
    }

    #[test]
    fn a_ring_is_zero_to_one() {
        let ring = |value: f64, text: &str| {
            in_closed(json!({ "type": "ring", "value": value, "text": text }))
        };
        for value in [0.0, 1.0] {
            assert!(Content::parse(ring(value, "100%")).is_ok());
        }
        assert_eq!(
            error(ring(1.01, "a")),
            "closed.left: value must be 0 to 1, not 1.01"
        );
        assert!(Content::parse(ring(-0.01, "a")).is_err());
        assert_eq!(
            error(ring(0.5, "1234567")),
            "closed.left: text is 7 wide, at most 6"
        );
    }

    #[test]
    fn cells_fit_in_three_rows_of_twenty() {
        let cells = |count: usize, columns: usize| {
            in_open(
                json!({ "type": "cells", "colors": vec![Value::Null; count], "columns": columns }),
            )
        };
        for (count, columns) in [(60, 20), (3, 1), (5, 15)] {
            assert!(
                Content::parse(cells(count, columns)).is_ok(),
                "{count} {columns}"
            );
        }
        assert_eq!(
            error(cells(61, 20)),
            "open.left[0]: 61 cells in 20 columns take 4 rows, at most 3"
        );
        assert!(Content::parse(cells(4, 1)).is_err());
        for columns in [0, 21] {
            assert_eq!(
                error(cells(1, columns)),
                format!("open.left[0]: columns must be 1 to 20, not {columns}")
            );
        }
        assert_eq!(error(cells(0, 5)), "open.left[0]: colors is empty");
        // current 要落在格子里
        let current = |current: usize| {
            in_open(
                json!({ "type": "cells", "colors": [null, null], "columns": 2, "current": current }),
            )
        };
        assert!(Content::parse(current(1)).is_ok());
        assert_eq!(error(current(2)), "open.left[0]: current must be below 2");
    }

    #[test]
    fn strings_cannot_be_empty_or_break_lines() {
        let text = |text: Value| in_open(json!({ "type": "text", "text": text }));
        for bad in [
            json!(""),
            json!([]),
            json!(["a", ""]),
            json!("a\nb"),
            json!([{ "text": "a\tb" }]),
            // 不是控制字符，但会换行
            json!("a\u{2028}b"),
        ] {
            assert!(
                error(text(bad.clone())).starts_with("open.left[0]: text "),
                "{bad}"
            );
        }
        let id = |id: &str| in_open(json!({ "type": "button", "id": id, "icon": "Pause" }));
        assert!(Content::parse(id(&"a".repeat(64))).is_ok());
        assert_eq!(
            error(id(&"a".repeat(65))),
            "open.left[0]: id is 65 bytes, at most 64"
        );
        assert_eq!(error(id("")), "open.left[0]: id is empty");
    }
}
