//! Data for the Daily, Weekly and Monthly pages: time per category, app rankings, and the details
//! shown after opening an app.
//!
//! The Daily, Weekly and Monthly pages share this code. Each file has the following role:
//! - `by_hour.rs`: splits activity records by hour to calculate each hour's time per category
//!   and the app ranking for a selected hour, for the Daily page.
//! - `by_date.rs`: queries daily time per category and app rankings within a date range for the
//!   Daily, Weekly and Monthly pages.
//! - `app_detail.rs`: queries the details shown after clicking an app, for daily, weekly and
//!   monthly time ranges.
//! - `time.rs`: provides helpers for parsing times, splitting time ranges by hour, and calculating
//!   date ranges.
//!
//! Every query must do two things: follow [`DeviceFilter`] to add up all devices or show one;
//! leave out "Ignored windows" (`a.excluded = 0`).

use serde::Serialize;

mod app_detail;
mod by_date;
mod by_hour;
#[cfg(test)]
mod test_seed;
mod time;

pub use app_detail::{app_range_detail, BucketBy};
pub use by_date::{day_category_time, top_apps};
pub use by_hour::{day_hour_apps, day_hours};
pub use time::{day_date, month_range, week_range};

/// One category's time in an hour or a day.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryTime {
    pub category_id: String,
    /// Seconds before rounding.
    pub secs: u64,
}

/// Each category's time in one hour.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourSlot {
    /// 0..=23
    pub hour: u8,
    /// Sorted by `secs`, most first; empty when the hour has no activity.
    pub segments: Vec<CategoryTime>,
}

/// One day's time per category. Used by the per-day heat map on the Weekly and Monthly pages.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaySummary {
    /// Date as `YYYY-MM-DD`
    pub date: String,
    /// The day's time split by category, sorted by `secs`, most first
    pub segments: Vec<CategoryTime>,
}

/// One app's total usage (one row in the top apps list).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    /// App name shown to the user: the group's display name, or the process name when ungrouped.
    pub display_name: String,
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
    /// Converts a command's optional device ID into a filter. `None`, empty strings and strings
    /// containing only whitespace select all devices. Other IDs are preserved as supplied.
    pub fn from_option(id: Option<String>) -> Self {
        match id {
            None => Self::All,
            Some(s) if s.trim().is_empty() => Self::All,
            Some(s) => Self::Only(s),
        }
    }

    /// Adds the device condition to the SQL, if there is one.
    /// The activities table must be aliased `a`. When the clause contains `?`, bind
    /// [`Self::sql_param`] at that placeholder's position in the SQL parameter list.
    pub(crate) fn sql_clause(&self) -> &'static str {
        match self {
            DeviceFilter::All => "",
            DeviceFilter::Only(_) => " AND a.device_id = ? ",
        }
    }

    /// The value for the `?` placeholder in [`Self::sql_clause`], or `None` for all devices.
    pub(crate) fn sql_param(&self) -> Option<&String> {
        match self {
            DeviceFilter::All => None,
            DeviceFilter::Only(id) => Some(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测 [`DeviceFilter::from_option`]：None / 空串 / 全空白 → All；非空 → Only。
    /// 这是所有报表 Tauri 命令入口的参数规整口径，错了会把"全设备"误当成某台设备查。
    #[test]
    fn from_option_normalizes_device_filter() {
        assert!(matches!(DeviceFilter::from_option(None), DeviceFilter::All));
        assert!(matches!(
            DeviceFilter::from_option(Some(String::new())),
            DeviceFilter::All
        ));
        // 全空白（含 tab）也归 All——前端 select 未选中时可能传占位空白串
        assert!(matches!(
            DeviceFilter::from_option(Some("  \t ".into())),
            DeviceFilter::All
        ));
        match DeviceFilter::from_option(Some("device-win".into())) {
            DeviceFilter::Only(id) => assert_eq!(id, "device-win"),
            DeviceFilter::All => panic!("非空 id 应得到 Only，不是 All"),
        }
        // 两端带空白但中间非空 → 保留原串的 Only（当前契约：只用 trim 判空，不改写 id）
        match DeviceFilter::from_option(Some(" dev ".into())) {
            DeviceFilter::Only(id) => assert_eq!(id, " dev ", "id 原样保留，不做 trim 改写"),
            DeviceFilter::All => panic!("含非空白字符的串不该归 All"),
        }
    }
}
