//! 定时补识别:每天到用户设定的时刻,自动批量识别堆积的未处理截图。
//!
//! 与「常驻 OCR」**正交**——这是独立的第三条消化路径(手动回填/常驻之外):
//! - 常驻开着:积压近零,到点自然秒过,互不打扰;
//! - 常驻关着:这是唯一的自动路径,给"不想 400MB 常驻、但要索引齐"的用户。
//!
//! 语义(与 [`crate::ai::auto_summary`] 同哲学):
//! - `settings.memory_ocr_daily_times` 非空即启用(可配多个点,如 12:00/23:00);
//!   空(默认)本模块沉睡;旧单时刻字段经 effective_ocr_daily_times 兼容;
//! - 到点后的下一轮检查触发(检查间隔 [`CHECK_GAP_SECS`],最多迟到一轮);
//! - 当天错过(未开机)下次启动后照常补;每天至多**尝试**一次,失败不重试
//!   (失败通常是模型缺失/磁盘问题,盲目重试只会烧 CPU 刷日志);
//! - 跑之前先幂等回填登记簿(主库截图全集为准),再清到空;
//! - **与常驻开关无关**(常驻开着到点也跑,积压近零则静默秒过);
//!   与手动/常驻/总结前清积压共用 digest 全局单批互斥,正在跑才让路等下轮。

use chrono::Local;
use tauri::{AppHandle, Manager};

use tauri::Emitter;

use super::digest;
use crate::commands::screen_memory::MemoryState;
use crate::repo::settings;
use crate::storage::{DbPool, SqliteResultExt};

/// 定时批开跑时发给前端的事件;前端据此弹系统通知(文案走 i18n)。
/// payload: `{ "pending": u64 }`。
pub const SCHEDULED_OCR_STARTED_EVENT: &str = "memory://scheduled-ocr-started";

/// 启动后首查延迟(秒):避开启动期的采集初始化。
const FIRST_CHECK_DELAY_SECS: u64 = 90;
/// 常规检查间隔(秒)。10 分钟:到点后最多迟到一轮,"定点"体感足够。
const CHECK_GAP_SECS: u64 = 10 * 60;

/// 后台调度任务。app 退出随进程终止。
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(FIRST_CHECK_DELAY_SECS)).await;
        loop {
            if let Err(e) = check_once(&app).await {
                log::debug!("定时补识别本轮跳过: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(CHECK_GAP_SECS)).await;
        }
    });
}

async fn check_once(app: &AppHandle) -> crate::error::Result<()> {
    let pool = app.state::<DbPool>();
    let cfg = settings::load(&pool).await?;

    let mem_state = app.state::<MemoryState>();
    let Some(mem) = mem_state.0.as_ref() else {
        return Ok(()); // 屏幕记忆库不可用:静默沉睡
    };

    let now = Local::now();
    let today = now.date_naive().to_string();
    let times = cfg.effective_ocr_daily_times();
    // 记账持久在 memory.sqlite(重启不丢——曾因只存内存,dev 重启一次就
    // 重跑一次、通知连环弹;每个点每天至多一次必须跨进程成立)。
    let attempted = load_marks(mem, &today).await?;
    // 本轮到期的点(可能多个:关机错过后补开机)。一批消化对所有到期点生效,
    // 跑一次、全部记账——连跑 N 次只会让后 N-1 次空转。
    let due = due_times(now.time(), &times, |t| {
        attempted.iter().any(|k| k == &attempt_key(&today, t))
    });
    // 冷却中 = 识别引擎刚被判定不可用:直接让路,**不标记当天已试**——
    // 定时点每天只有一次机会,不能烧在一个注定失败的批上
    if due.is_empty() || digest::is_running() || digest::cooldown_remaining_secs().is_some() {
        return Ok(());
    }

    // 幂等回填:主库有截图、登记簿没登的行先补上。
    // 注意此时还没标记"今天试过"——回填/计数期间不消耗当天机会。
    digest::backfill_from_activities(&pool, mem).await?;
    let pending = super::frames::count_pending(mem).await.unwrap_or(0);
    if pending == 0 {
        log::debug!("定时补识别:到点但无积压,今天无事可做");
        return Ok(());
    }

    // 回填可能耗时,紧贴开跑前把互斥再验一遍;在跑 = 不标记不通知,
    // 下轮(10 分钟)自然重试。此后才标记+通知+跑——run 的任何失败一律
    // 当天不重试(简单一条规则,无撤销路径)。
    if digest::is_running() || digest::cooldown_remaining_secs().is_some() {
        log::debug!("定时补识别:别的消化批正在跑,让路下轮");
        return Ok(());
    }
    save_marks(mem, &today, &due).await?;

    log::info!(
        "定时补识别:到点({}),{pending} 帧待识别,开始清积压",
        due.join("/")
    );
    // 系统级提示由前端弹(文案走界面语言的 i18n);窗口收进托盘时监听仍在
    let _ = app.emit(
        SCHEDULED_OCR_STARTED_EVENT,
        serde_json::json!({ "pending": pending }),
    );
    match digest::run(mem).await {
        Ok(report) => log::info!("定时补识别完成: {report:?}"),
        // 被拒 = 批根本没开跑(上面的 is_running 检查与 run 内部抢权之间,
        // 别的批可能抢先;或恰好进入冷却)。必须退还当天标记——定时点每天
        // 只有一次机会,不能被一次没发生的运行消耗掉。
        // 真正开跑后的失败仍不退:当天不重试是既定规则。
        Err(e)
            if matches!(e, crate::error::Error::InvalidInput(_))
                || e.to_string().contains("冷却") =>
        {
            log::debug!("定时补识别:抢批失败({e}),退还当天标记,下轮再试");
            remove_marks(mem, &today, &due).await?;
        }
        Err(e) => log::warn!("定时补识别失败(今天不再重试): {e}"),
    }
    Ok(())
}

/// 退还一组当天标记([`save_marks`] 的逆操作;抢批失败时用)。
async fn remove_marks(
    mem: &super::MemoryDb,
    date: &str,
    due: &[String],
) -> crate::error::Result<()> {
    let keys: Vec<String> = due.iter().map(|t| attempt_key(date, t)).collect();
    mem.0
        .call(move |conn| {
            for k in &keys {
                conn.execute("DELETE FROM scheduled_ocr_marks WHERE mark = ?1", [k])
                    .db()?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// 每点每天一次的记账键。
fn attempt_key(date: &str, time: &str) -> String {
    format!("{date}@{time}")
}

/// 读今天已跑过的记账(持久层:memory.sqlite `scheduled_ocr_marks`)。
async fn load_marks(mem: &super::MemoryDb, date: &str) -> crate::error::Result<Vec<String>> {
    let prefix = format!("{date}@%");
    let rows = mem
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare("SELECT mark FROM scheduled_ocr_marks WHERE mark LIKE ?1")
                .db()?;
            let out = stmt
                .query_map([prefix], |r| r.get::<_, String>(0))
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(out)
        })
        .await?;
    Ok(rows)
}

/// 写本轮消耗的记账,并顺手清掉非今天的旧行(表恒只有当天数据,不用独立清理任务)。
async fn save_marks(mem: &super::MemoryDb, date: &str, due: &[String]) -> crate::error::Result<()> {
    let date = date.to_string();
    let keys: Vec<String> = due.iter().map(|t| attempt_key(&date, t)).collect();
    let prefix = format!("{date}@%");
    mem.0
        .call(move |conn| {
            conn.execute(
                "DELETE FROM scheduled_ocr_marks WHERE mark NOT LIKE ?1",
                [&prefix],
            )
            .db()?;
            for k in &keys {
                conn.execute(
                    "INSERT OR IGNORE INTO scheduled_ocr_marks(mark) VALUES(?1)",
                    [k],
                )
                .db()?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// 本轮到期的时间点(纯函数,单测覆盖):已到点且今天没试过的全部点,
/// **保持配置(添加)顺序**。与常驻开关无关(产品定稿:常驻开着到点也跑);
/// 坏格式项跳过(sanitize 层本应拦住,这里 fail-safe 不炸整表)。
fn due_times(
    now: chrono::NaiveTime,
    configured: &[String],
    attempted: impl Fn(&str) -> bool,
) -> Vec<String> {
    configured
        .iter()
        .filter(|raw| {
            let Ok(at) = chrono::NaiveTime::parse_from_str(raw.trim(), "%H:%M") else {
                return false;
            };
            now >= at && !attempted(raw)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    /// due_times 真值表:到点 + 未试过才入列,顺序保持配置序,坏格式跳过。
    /// 注意没有"常驻开关"条件——定时批与常驻正交。
    #[test]
    fn due_times_truth_table() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        let times = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let none_attempted = |_: &str| false;

        // 空配置:永不触发(功能关闭态)
        assert!(due_times(t(23, 59), &[], none_attempted).is_empty());
        // 单点:到点(含恰好)入列,未到不入
        assert_eq!(
            due_times(t(3, 0), &times(&["03:00"]), none_attempted),
            times(&["03:00"])
        );
        assert!(due_times(t(2, 59), &times(&["03:00"]), none_attempted).is_empty());
        // 多点:只收已到期的,顺序 = 配置(添加)顺序而非时刻序
        assert_eq!(
            due_times(
                t(13, 0),
                &times(&["23:00", "12:00", "09:00"]),
                none_attempted
            ),
            times(&["12:00", "09:00"])
        );
        // 已试过的点被排除(逐点记账)
        let attempted_12 = |k: &str| k == "12:00";
        assert_eq!(
            due_times(t(13, 0), &times(&["12:00", "09:00"]), attempted_12),
            times(&["09:00"])
        );
        // 坏格式项跳过,不炸整表
        assert_eq!(
            due_times(t(13, 0), &times(&["25:99", "12:00"]), none_attempted),
            times(&["12:00"])
        );
        // 空白容忍
        assert_eq!(
            due_times(t(9, 30), &times(&[" 09:30 "]), none_attempted),
            times(&[" 09:30 "])
        );
    }

    /// 持久记账:写入可读回、幂等、跨"重启"(同一库新查询)仍在、
    /// 写新日期时旧日期行被顺手清掉——通知连环弹的根因回归。
    #[tokio::test]
    async fn marks_persist_and_prune_old_dates() {
        let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
        let day1 = "2026-08-01";
        let day2 = "2026-08-02";

        assert!(load_marks(&mem, day1).await.unwrap().is_empty());

        save_marks(&mem, day1, &["23:00".to_string(), "12:00".to_string()])
            .await
            .unwrap();
        let got = load_marks(&mem, day1).await.unwrap();
        assert_eq!(got.len(), 2, "两点各一行");
        assert!(got.contains(&"2026-08-01@23:00".to_string()));

        // 幂等:重复写同键不报错不加行
        save_marks(&mem, day1, &["23:00".to_string()])
            .await
            .unwrap();
        assert_eq!(load_marks(&mem, day1).await.unwrap().len(), 2);

        // 次日写入:昨天的行被清,今天的立住——表恒只有当天数据
        save_marks(&mem, day2, &["23:00".to_string()])
            .await
            .unwrap();
        assert!(
            load_marks(&mem, day1).await.unwrap().is_empty(),
            "旧日期应被清"
        );
        assert_eq!(load_marks(&mem, day2).await.unwrap().len(), 1);
    }

    /// 抢批失败要退还标记:定时点每天一次机会,不能被"没发生的运行"消耗。
    /// remove_marks 只退指定点,不误伤同日其它点。
    #[tokio::test]
    async fn remove_marks_returns_only_the_given_points() {
        let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
        let day = "2026-08-05";
        save_marks(&mem, day, &["12:00".to_string(), "22:00".to_string()])
            .await
            .unwrap();

        remove_marks(&mem, day, &["22:00".to_string()]).await.unwrap();
        let left = load_marks(&mem, day).await.unwrap();
        assert_eq!(left, vec!["2026-08-05@12:00".to_string()], "只退 22:00,12:00 保留");

        // 幂等:退不存在的键不报错
        remove_marks(&mem, day, &["22:00".to_string()]).await.unwrap();
        assert_eq!(load_marks(&mem, day).await.unwrap().len(), 1);
    }

    /// 记账键:date@time,同天不同点互不影响。
    #[test]
    fn attempt_key_is_per_date_and_time() {
        assert_eq!(attempt_key("2026-08-01", "12:00"), "2026-08-01@12:00");
        assert_ne!(
            attempt_key("2026-08-01", "12:00"),
            attempt_key("2026-08-01", "23:00")
        );
    }
}
