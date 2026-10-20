//! Statistics methods (`stats.*`).

use std::collections::HashMap;

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::categories::CategoryRef;
use super::{check_granted, has_permission};
use crate::error::{Error, Result};
use crate::repo::plugins::Plugin;
use crate::repo::reports::{day_category_time, month_range, week_range, DeviceFilter};
use crate::storage::DbPool;

/// `stats.day_by_category`: how long each category was used on `date`, longest first.
///
/// - `device`: the device selected in the main window. It is used only when the `devices`
///   parameter is `"selected"`.
/// - `self_id`: this computer's device id.
///
/// # Errors
///
/// - The parameters are wrong, or `date` is after today: [`Error::InvalidInput`]
/// - The plugin has no access to this day, or it asks for the devices selected in the main
///   window without `devices.read`: [`Error::PluginNoPermission`]
pub(super) async fn day_by_category(
    pool: &DbPool,
    plugin: &Plugin,
    params: Value,
    device: Option<String>,
    self_id: &str,
) -> Result<Value> {
    let DayParams { date, devices } = serde_json::from_value(params).map_err(|_| {
        Error::InvalidInput(
            r#"stats.day_by_category needs { date: "YYYY-MM-DD", devices?: "this" | "selected" }"#,
        )
    })?;
    let today = Local::now().date_naive();
    if date > today {
        return Err(Error::InvalidInput(
            "stats.day_by_category cannot read a day after today",
        ));
    }
    check_range(plugin, date, today)?;
    let device = match devices {
        Devices::This => DeviceFilter::Only(self_id.into()),
        Devices::Selected => {
            check_granted(plugin, "devices.read")?;
            selected_device(device, self_id)
        }
    };
    let categories: HashMap<String, CategoryRef> = CategoryRef::list(pool)
        .await?
        .into_iter()
        .map(|c| (c.id.clone(), c))
        .collect();
    let days = day_category_time(pool, date, date, device).await?;
    let items: Vec<CategorySeconds> = days
        .into_iter()
        .flat_map(|d| d.segments)
        .map(|s| {
            // Missing only if the category was deleted between the queries
            let category = categories
                .get(&s.category_id)
                .cloned()
                .unwrap_or(CategoryRef {
                    id: s.category_id,
                    ..Default::default()
                });
            CategorySeconds {
                category,
                seconds: s.secs,
            }
        })
        .collect();
    Ok(serde_json::to_value(items)?)
}

/// The parameters of `stats.day_by_category`.
#[derive(Deserialize)]
struct DayParams {
    date: NaiveDate,
    #[serde(default)]
    devices: Devices,
}

/// The devices to query.
#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Devices {
    /// This computer. Needs no other permission
    #[default]
    This,
    /// The devices selected in the main window. Needs `devices.read`
    Selected,
}

/// One item returned by `stats.day_by_category`: the category's fields and `seconds`, side by side.
#[derive(Debug, PartialEq, Serialize)]
struct CategorySeconds {
    #[serde(flatten)]
    category: CategoryRef,
    seconds: u64,
}

/// Checks that the plugin's range can read the day `date`.
///
/// # Errors
///
/// - It cannot: [`Error::PluginNoPermission`], holding the narrowest range that can read this day
fn check_range(plugin: &Plugin, date: NaiveDate, today: NaiveDate) -> Result<()> {
    let (monday, _) = week_range(today, 0);
    let (first_of_month, _) = month_range(today, 0);
    // From narrowest to widest: the earliest day each range can read
    let ranges = [
        ("stats.today", today),
        ("stats.week", monday),
        // The month range also covers this week: in the first days of a month, this week starts
        // in the previous month
        ("stats.month", first_of_month.min(monday)),
        ("stats.history", NaiveDate::MIN),
    ];
    if ranges
        .iter()
        .any(|(name, earliest)| date >= *earliest && has_permission(plugin, name))
    {
        return Ok(());
    }
    let (needed, _) = ranges
        .iter()
        .find(|(_, earliest)| date >= *earliest)
        .expect("the last range should cover every day");
    Err(Error::PluginNoPermission((*needed).into()))
}

/// Turns the device selected in the main window into a query filter. Never selected (`None`)
/// and `"self"`, left by older versions, mean this computer, as in the main window.
fn selected_device(device: Option<String>, self_id: &str) -> DeviceFilter {
    match device.as_deref() {
        Some("all") => DeviceFilter::All,
        None | Some("" | "self") => DeviceFilter::Only(self_id.into()),
        Some(id) => DeviceFilter::Only(id.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::plugin_with_permissions;
    use crate::repo::reports::test_seed::{insert_activity, seed_solo_group};
    use crate::repo::test_util::fresh_test_pool;
    use crate::repo::{categories, super_categories};

    const SELF_ID: &str = "this-mac";

    fn today() -> String {
        Local::now().date_naive().to_string()
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    /// 主界面选的是 `ui_device`。
    async fn day(
        pool: &DbPool,
        permissions: &[&str],
        params: Value,
        ui_device: Option<&str>,
    ) -> Result<Value> {
        let plugin = plugin_with_permissions("focus", permissions);
        day_by_category(pool, &plugin, params, ui_device.map(String::from), SELF_ID).await
    }

    /// 返回 id、库里的名字、颜色、图标、秒数和所在的大类，用时多的在前；没归入大类的是 null。
    /// 插件按这些键取值，返回的 JSON 整个锁在这里，改了已发布的插件就读错。
    #[tokio::test]
    async fn day_by_category_returns_names_seconds_and_super_category() {
        let pool = fresh_test_pool().await;
        super_categories::assign_category(&pool, "code", Some("work".into()))
            .await
            .unwrap();
        let job_hunting = categories::create(
            &pool,
            categories::CategoryInput {
                name: "求职".into(),
                color: "#000000".into(),
                icon: "Briefcase".into(),
            },
        )
        .await
        .unwrap();
        seed_solo_group(&pool, "Code", "code").await;
        seed_solo_group(&pool, "LinkedIn", &job_hunting.id).await;
        insert_activity(&pool, SELF_ID, &today(), "Code", 600).await;
        insert_activity(&pool, SELF_ID, &today(), "LinkedIn", 1200).await;

        let got = day(
            &pool,
            &["stats.read", "stats.today"],
            serde_json::json!({ "date": today() }),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            got,
            serde_json::json!([
                {
                    "id": job_hunting.id,
                    "name": "求职",
                    "color": "#000000",
                    "icon": "Briefcase",
                    "seconds": 1200,
                    "superCategory": null,
                },
                {
                    "id": "code",
                    "name": "编程",
                    "color": "#a78bfa",
                    "icon": "Code",
                    "seconds": 600,
                    "superCategory": {
                        "id": "work",
                        "name": "工作",
                        "color": "#8b5cf6",
                        "icon": "Briefcase",
                    },
                },
            ])
        );
    }

    /// 不传 devices 只算本机，有 devices.read 也一样；"selected" 按主界面选的设备算，
    /// 没有 devices.read 就报错。
    #[tokio::test]
    async fn day_by_category_counts_the_devices_the_plugin_asks_for() {
        let pool = fresh_test_pool().await;
        // 今天 Code 在本机用了 600 秒，在另一台电脑上用了 1200 秒
        seed_solo_group(&pool, "Code", "code").await;
        insert_activity(&pool, SELF_ID, &today(), "Code", 600).await;
        insert_activity(&pool, "other-pc", &today(), "Code", 1200).await;
        let with_devices = ["stats.read", "stats.today", "devices.read"];
        let seconds = |v: Value| v[0]["seconds"].as_u64().unwrap();

        let this = serde_json::json!({ "date": today() });
        assert_eq!(
            seconds(day(&pool, &with_devices, this, Some("all")).await.unwrap()),
            600
        );

        let selected = serde_json::json!({ "date": today(), "devices": "selected" });
        for (ui_device, expected) in [
            (Some("all"), 1800),
            (Some("other-pc"), 1200),
            (Some("self"), 600),
            (None, 600),
        ] {
            let got = day(&pool, &with_devices, selected.clone(), ui_device).await;
            assert_eq!(seconds(got.unwrap()), expected, "{ui_device:?}");
        }

        let err = day(&pool, &["stats.read", "stats.today"], selected, Some("all"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::PluginNoPermission(ref got) if got == "devices.read"),
            "{err}"
        );
    }

    /// 没有日期、日期格式不对、日期在今天之后、devices 不认识：都报参数错误。
    #[tokio::test]
    async fn day_by_category_rejects_bad_params() {
        let pool = fresh_test_pool().await;
        let permissions = ["stats.read", "stats.history"];
        let tomorrow = Local::now().date_naive().succ_opt().unwrap();
        for params in [
            Value::Null,
            serde_json::json!({ "date": "2026/10/19" }),
            serde_json::json!({ "date": tomorrow }),
            serde_json::json!({ "date": NaiveDate::MAX }),
            serde_json::json!({ "date": today(), "devices": "all" }),
        ] {
            let err = day(&pool, &permissions, params.clone(), None)
                .await
                .unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{params}: {err}");
        }
    }

    /// 一档包一档；读不到时报能读到这天的最窄的一档。
    #[test]
    fn check_range_follows_the_ranges() {
        // 10 月 1 日是周四，本周从 9 月 28 日开始，跨到了上个月；10 月 19 日是周一
        // (今天, 插件的档位, 要读的日子, 结果)，档位名省掉了前缀 stats.
        let cases = [
            ("2026-10-01", "today", "2026-10-01", Ok(())),
            ("2026-10-01", "today", "2026-09-30", Err("week")),
            ("2026-10-01", "week", "2026-09-28", Ok(())),
            ("2026-10-01", "month", "2026-09-28", Ok(())),
            ("2026-10-01", "month", "2026-09-27", Err("history")),
            ("2026-10-19", "week", "2026-10-18", Err("month")),
            ("2026-10-19", "month", "2026-10-01", Ok(())),
            ("2026-10-19", "month", "2026-09-30", Err("history")),
            ("2026-10-19", "history", "2025-01-01", Ok(())),
        ];
        for (today, range, day, expected) in cases {
            let range = format!("stats.{range}");
            let plugin = plugin_with_permissions("focus", &["stats.read", &range]);
            let got = check_range(&plugin, date(day), date(today));
            match expected {
                Ok(()) => assert!(got.is_ok(), "{today} {range} {day}: {got:?}"),
                Err(needed) => assert!(
                    matches!(got, Err(Error::PluginNoPermission(ref p)) if *p == format!("stats.{needed}")),
                    "{today} {range} {day}: {got:?}"
                ),
            }
        }
    }
}
