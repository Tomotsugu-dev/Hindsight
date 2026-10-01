//! Data for the Daily, Weekly and Monthly pages: time per category, app rankings, and the details
//! shown after opening an app.
//!
//! Files:
//! - `day.rs`: Daily.
//! - `range.rs`: Weekly and Monthly, queried by start and end date.
//! - `app_detail.rs`: details after opening an app, for day, week and month.
//! - `time.rs`: time helpers shared by `day.rs` and `app_detail.rs`.
//!
//! Every query must do two things: follow [`DeviceFilter`] to add up all devices or show one;
//! leave out "Ignored windows" (`a.excluded = 0`).

use serde::Serialize;

mod app_detail;
mod day;
mod range;
#[cfg(test)]
mod test_seed;
mod time;

pub use app_detail::{app_day_detail, app_month_detail, app_week_detail};
pub use day::{day_apps, day_hour_apps, day_hours};
pub use range::{month_apps, month_days, week_apps, week_days};

// TODO: Rename to `CategorySegment`. `DaySummary` also uses it for each category's time in a day,
// not only in an hour. The frontend has a type with the same name in `src/api/hindsight.ts`;
// rename both.
/// One category's time in a bucket (an hour or a day).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourSegment {
    pub category_id: String,
    /// Minutes after rounding, for bar charts only. Each bucket is rounded on its own, so adding
    /// them drifts from the real total; add up `secs` instead. An hour bucket can exceed 60 when
    /// devices are combined.
    // TODO: Remove this field and send only `secs`. The Daily, Weekly and Monthly bar charts on
    // the frontend (HourlyChart, WeeklyBarChart, DailyBarChart) all add it up; change them to add
    // up `secs` and convert to minutes at the end. Sort categories by `secs` in `day.rs` and
    // `range.rs` as well.
    pub minutes: u32,
    /// Seconds before rounding.
    pub secs: u64,
}

/// Each category's time in one hour.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourSlot {
    /// 0..=23
    pub hour: u8,
    /// Sorted by `minutes`, most first; empty when the hour has no activity.
    pub segments: Vec<HourSegment>,
}

/// One day's time per category. Used by the per-day heat map on the Weekly and Monthly pages.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaySummary {
    /// Date as `YYYY-MM-DD`
    pub date: String,
    /// The day's minutes, split by category
    pub segments: Vec<HourSegment>,
}

/// One app's total usage (one row in the top apps list).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    /// Display name: the group's display_name (e.g. "Visual Studio Code"); members of one group
    /// are merged into one row
    // TODO: Rename to `display_name`. It holds the group's display name, not a process name; the
    // process name is in `icon_process`. This is the key sent to the frontend, so also change the
    // type in `src/api/hindsight.ts` and the code that reads it (usePeriodRankings, PieDrillDetail,
    // usageExport).
    pub process: String,
    pub category_id: String,
    pub minutes: u32,
    /// The process_name AppIcon uses to look up the icon: a stable member name picked from the
    /// merged group, so the frontend can look it up with any process_name in the group (icons are
    /// synced across devices).
    pub icon_process: String,
}

/// Data for the details drawer opened by clicking an app: a row of time bars, plus window titles
/// ranked by time. Shared by day, week and month; each range is added up in the backend before
/// it is sent, so a month's tens of thousands of raw sessions are not all sent to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDetail {
    /// Time bars: by hour, 24 bars (key = "0".."23"); by day, one bar per day in the range
    /// (key = "YYYY-MM-DD"). Already in time order, with empty buckets as 0, ready to draw.
    pub buckets: Vec<DetailBucket>,
    /// Time per window title, most first. Raw titles; the frontend strips the app name suffix and
    /// merges them.
    pub titles: Vec<TitleUsage>,
    /// Whether the representative process is a browser; the frontend uses it to decide whether to
    /// group the titles by site.
    pub is_browser: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailBucket {
    /// By hour: "0".."23"; by day: "YYYY-MM-DD"
    pub key: String,
    pub secs: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleUsage {
    pub title: String,
    pub secs: u32,
    /// The site of a browser session (e.g. github.com). None for other apps, old records, or when
    /// the address bar can't be read. The frontend uses it to put page rows into the "By site"
    /// groups.
    pub host: Option<String>,
}

/// Which devices a report covers: All = add up all devices, Only(id) = one device only
#[derive(Debug, Clone)]
pub enum DeviceFilter {
    All,
    Only(String),
}

impl DeviceFilter {
    /// Adds the device condition to the SQL, if there is one
    pub(crate) fn sql_clause(&self) -> &'static str {
        match self {
            DeviceFilter::All => "",
            DeviceFilter::Only(_) => " AND a.device_id = ? ",
        }
    }

    /// Works with [`DeviceFilter::sql_clause`] to give the prepared statement its extra parameter,
    /// if there is one.
    // TODO: Rename to `sql_param` so it reads as a pair with `sql_clause`: it is the value for the
    // `?` in that fragment. Add two rules to the `sql_clause` doc: the activities table must be
    // aliased `a`; the parameter must go into the list at the fragment's position in the SQL.
    // Other callers are in `ai/summary_operations.rs` and `repo/ai_summaries.rs`; change them too.
    pub(crate) fn extra_param(&self) -> Option<&String> {
        match self {
            DeviceFilter::All => None,
            DeviceFilter::Only(id) => Some(id),
        }
    }
}

/// Turns the `Option<String>` device filter from a Tauri command into a [`DeviceFilter`].
/// `None` / empty / all whitespace → All; any other string → Only.
// TODO: Move into `impl DeviceFilter` as `DeviceFilter::from_option`. Not `impl From`: an empty
// string becomes All, and `.into()` would hide that. Callers are in `commands/data.rs` and
// `commands/ai_summary.rs`.
pub fn device_filter_from_option(id: Option<String>) -> DeviceFilter {
    match id {
        None => DeviceFilter::All,
        Some(s) if s.trim().is_empty() => DeviceFilter::All,
        Some(s) => DeviceFilter::Only(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测 [`device_filter_from_option`]：None / 空串 / 全空白 → All；非空 → Only。
    /// 这是所有报表 Tauri 命令入口的参数规整口径，错了会把"全设备"误当成某台设备查。
    #[test]
    fn device_filter_from_option_normalizes() {
        assert!(matches!(device_filter_from_option(None), DeviceFilter::All));
        assert!(matches!(
            device_filter_from_option(Some(String::new())),
            DeviceFilter::All
        ));
        // 全空白（含 tab）也归 All——前端 select 未选中时可能传占位空白串
        assert!(matches!(
            device_filter_from_option(Some("  \t ".into())),
            DeviceFilter::All
        ));
        match device_filter_from_option(Some("device-win".into())) {
            DeviceFilter::Only(id) => assert_eq!(id, "device-win"),
            DeviceFilter::All => panic!("非空 id 应得到 Only，不是 All"),
        }
        // 两端带空白但中间非空 → 保留原串的 Only（当前契约：只用 trim 判空，不改写 id）
        match device_filter_from_option(Some(" dev ".into())) {
            DeviceFilter::Only(id) => assert_eq!(id, " dev ", "id 原样保留，不做 trim 改写"),
            DeviceFilter::All => panic!("含非空白字符的串不该归 All"),
        }
    }
}
