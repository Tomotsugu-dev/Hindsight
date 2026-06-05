//! 周报编排（单步纯文本）。
//!
//! 跟 [`crate::ai::summary_runner::DaySummaryRunner`] 的关键差异：
//! - **不过 step 1**：原料是 `ai_summaries` 表里 `source='daily'` 的段总结文本，
//!   不再过 vision LLM 看图；一次纯文本 chat 就够。
//! - **没有"段"概念**：一周 = 一行 `ai_summaries`（`source='weekly'`，
//!   `local_date = 周一日期`，`segment_idx = 0`）。
//! - **进度事件极简**：只 emit `engine_starting` / `summarizing` / `segment_done` /
//!   `all_done` / `error` / `cancelled`，复用同一份 `SUMMARY_PROGRESS_EVENT` 通道，
//!   前端按 `source = "weekly"` 过滤跟 daily 互不干扰。
//!
//! 校验链：
//! 1. 必须有可用的 step2 模型（本地 summary main 非空 或 external_enabled=true）
//! 2. 严格模式（`allow_missing_days=false`）下：这一周内必须至少有一天的 daily
//!    段总结落库（`status = ok`），否则直接写 `status = error` 行让前端引导。
//!    宽松模式（前端 precheck → 用户在确认弹框点了"继续生成"）下：缺日报的天
//!    用当日 top apps 顶替进 prompt；整周无日报也允许跑——这时仅基于 weekly +
//!    每日 top apps 做简化分析。只有 days 和 top_apps **同时为空**时才视为
//!    "本周没用过电脑"写 error 兜底。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{Datelike, Duration, NaiveDate};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::ai::models;
use crate::ai::prompt::{
    build_weekly_system_prompt, build_weekly_user_prompt, weekday_short, WeeklyContext,
};
use crate::ai::server::{EngineStartOverrides, EngineState, EngineSupervisor};
use crate::ai::summary_operations::build_step2;
use crate::ai::summary_progress::{SummaryProgress, SUMMARY_PROGRESS_EVENT};
use crate::error::{Error, Result};
use crate::repo::ai_summaries::{self, SegmentSummaryRow};
use crate::repo::reports::DeviceFilter;
use crate::repo::settings as settings_repo;
use crate::storage::{utc_now_rfc3339, DbPool};

/// 周报固定的 source 命名空间值。
pub const WEEKLY_SOURCE: &str = "weekly";

/// 周报里那"唯一一段"的 segment_idx——主键三元组里固定填 0，避免后端读写时
/// 各处魔术数字散落。
const WEEKLY_SEGMENT_IDX: u32 = 0;

/// 单点入口：跑某一周的周报。lib.rs 通过命令体临时构造，每次 invoke 新建一份。
pub struct WeekSummaryRunner {
    pool: DbPool,
    supervisor: Arc<EngineSupervisor>,
    app: AppHandle,
    /// 取消信号——跟 daily 共用全局 [`crate::commands::ai_summary::SummaryCancel`]。
    /// 单次 chat 跑起来后没法中途中断，所以 cancel 实际只在"启动引擎前 / chat 调用前"
    /// 检查到时早 return。
    cancel: Arc<AtomicBool>,
}

impl WeekSummaryRunner {
    pub fn new(
        pool: DbPool,
        supervisor: Arc<EngineSupervisor>,
        app: AppHandle,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            pool,
            supervisor,
            app,
            cancel,
        }
    }

    /// 跑某一周的周报。
    ///
    /// `week_start` 必须是周一（调用方负责传对——`commands` 层会把任意日期对齐到周一）。
    /// `force_refresh = true` 时无视已有行直接重跑；false 且已有 ok 行时直接 `Ok(())` 早返回。
    /// `allow_missing_days = true` 时：
    ///   - 部分日缺日报：用当日 top apps 文本顶替进入 prompt
    ///   - 整周无日报但有 activity：仅基于整周 + 每日 top apps 做简化分析
    ///
    /// 不再因"日报缺失"早 return error。
    pub async fn run(
        &self,
        week_start: NaiveDate,
        force_refresh: bool,
        allow_missing_days: bool,
    ) -> Result<()> {
        let week_end = week_start + Duration::days(6);
        let week_key = week_start.format("%Y-%m-%d").to_string();
        let end_key = week_end.format("%Y-%m-%d").to_string();

        // 已有 ok 行 + 不强刷 → 直接返回让前端走"读 DB 路径"
        if !force_refresh && self.weekly_already_ok(&week_key).await? {
            self.emit_phase("all_done", &week_key, None, None);
            return Ok(());
        }

        let cfg = settings_repo::load(&self.pool).await?;
        let ai = cfg.ai.clone();

        // step2 模型校验：本地 summary main 非空 或 选定走云端
        if !ai.summary_use_cloud() && ai.effective_summary_main().trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」给段总结选一个模型，或选定云端 API 跑总结",
            ));
        }

        if force_refresh {
            ai_summaries::clear_day_summaries_only(&self.pool, WEEKLY_SOURCE, &week_key).await?;
        }

        // 拉一周内所有 daily 段总结，按日期 group
        let daily_rows = ai_summaries::get_range(&self.pool, "daily", &week_key, &end_key).await?;
        let lang = ai.prompt_language.as_str().to_string();
        let mut days = group_days(&daily_rows, week_start, week_end, &lang);

        // 严格模式（前端没确认 allow）下：整周无日报 = 老行为，写 error 行让前端引导
        // 用户先补日报。宽松模式继续往下走——通过补缺失日 fallback / 简化分析兜底。
        if days.is_empty() && !allow_missing_days {
            let row = SegmentSummaryRow {
                source: WEEKLY_SOURCE.to_string(),
                local_date: week_key.clone(),
                segment_idx: WEEKLY_SEGMENT_IDX,
                label: weekly_label(&week_key, &end_key),
                start_hour: 0,
                end_hour: 0,
                content: String::new(),
                model: String::new(),
                status: "error".to_string(),
                error: Some(
                    "本周还没有任何已生成的日报。请先到「日报」页生成几天后再试。".to_string(),
                ),
                generated_at: utc_now_rfc3339(),
            };
            ai_summaries::upsert_segment(&self.pool, &row).await?;
            // emit 一条 segment_done 让前端 store 写入这一行（status=error），
            // 紧接着 all_done 收尾——避免前端按钮一直转
            let mut p = SummaryProgress::base(
                WEEKLY_SOURCE.to_string(),
                week_key.clone(),
                "segment_done",
                1,
            );
            p.segment_idx = Some(WEEKLY_SEGMENT_IDX);
            p.status = Some("error");
            p.message = row.error.clone();
            self.emit(p);
            self.emit_phase("all_done", &week_key, None, None);
            return Ok(());
        }

        // 宽松模式：缺日报的天用当日 top apps 文本顶替——还是缺数据的天（既没日报
        // 也没 activity）直接不进 prompt，让 LLM 视为"未使用电脑"。
        if allow_missing_days {
            let present_dates: HashSet<String> = days.iter().map(|(d, _, _)| d.clone()).collect();
            let mut day = week_start;
            while day <= week_end {
                let day_str = day.format("%Y-%m-%d").to_string();
                if !present_dates.contains(&day_str) {
                    let day_apps = ai_summaries::list_range_top_apps(
                        &self.pool,
                        &day_str,
                        &day_str,
                        &ai.excluded_categories,
                        DeviceFilter::All,
                        8,
                    )
                    .await
                    .unwrap_or_else(|e| {
                        log::warn!("拉缺失日 top apps 失败（{}）：{e}", day_str);
                        Vec::new()
                    });
                    if !day_apps.is_empty() {
                        let text = format_missing_day_fallback(&lang, &day_apps);
                        let weekday = weekday_short(&lang, day.weekday());
                        days.push((day_str, weekday.to_string(), text));
                    }
                }
                day += Duration::days(1);
            }
            days.sort_by(|a, b| a.0.cmp(&b.0));
        }

        // 整周 top apps：跟 daily 段总结同语义（excluded_categories 过滤、按组合并），
        // 但跨 7 天聚合。device 用 All——周报当前没暴露设备维度，跟现有"多设备聚合"一致。
        // 查询失败不让整轮报错——top_apps 只是辅助信号，丢了就让 LLM 只看日报全文。
        // 提到引擎启动前查：宽松模式下没 activity 也要早 fail，省得白等模型加载。
        let top_apps = ai_summaries::list_range_top_apps(
            &self.pool,
            &week_key,
            &end_key,
            &ai.excluded_categories,
            DeviceFilter::All,
            8,
        )
        .await
        .unwrap_or_else(|e| {
            log::warn!("拉一周 top apps 失败（{}~{}）：{e}", week_key, end_key);
            Vec::new()
        });

        // 宽松模式终极兜底：日报也空 + 整周 top_apps 也空 = 本周根本没用电脑，没东西可写
        if allow_missing_days && days.is_empty() && top_apps.is_empty() {
            let row = SegmentSummaryRow {
                source: WEEKLY_SOURCE.to_string(),
                local_date: week_key.clone(),
                segment_idx: WEEKLY_SEGMENT_IDX,
                label: weekly_label(&week_key, &end_key),
                start_hour: 0,
                end_hour: 0,
                content: String::new(),
                model: String::new(),
                status: "error".to_string(),
                error: Some(
                    "本周既没有任何日报，也没有应用使用记录。请确认这一周是否有使用电脑。"
                        .to_string(),
                ),
                generated_at: utc_now_rfc3339(),
            };
            ai_summaries::upsert_segment(&self.pool, &row).await?;
            let mut p = SummaryProgress::base(
                WEEKLY_SOURCE.to_string(),
                week_key.clone(),
                "segment_done",
                1,
            );
            p.segment_idx = Some(WEEKLY_SEGMENT_IDX);
            p.status = Some("error");
            p.message = row.error.clone();
            self.emit(p);
            self.emit_phase("all_done", &week_key, None, None);
            return Ok(());
        }

        // 取消检查：启动引擎前
        if self.cancel.load(Ordering::Relaxed) {
            self.emit_phase("cancelled", &week_key, None, None);
            return Ok(());
        }

        // 本地引擎冷启动提示——supervisor.status() 不是 Running 时才推 engine_starting
        // 让前端显示"加载模型中…"。云端路径（summary_use_cloud()）跳过，不动 supervisor。
        let port = if !ai.summary_use_cloud() {
            let st = self.supervisor.status().await;
            if st.state != EngineState::Running {
                self.emit_phase(
                    "engine_starting",
                    &week_key,
                    None,
                    Some("加载模型中（首次约 30-90 秒）…".to_string()),
                );
            }
            self.ensure_engine_running(&ai).await?
        } else {
            // 云端路径用不到端口；step2 客户端会忽略它。占位 0。
            0
        };

        // 取消检查：chat 之前
        if self.cancel.load(Ordering::Relaxed) {
            self.emit_phase("cancelled", &week_key, None, None);
            return Ok(());
        }

        // 拼 prompt + 调 step2
        let ctx = WeeklyContext {
            week_start: &week_key,
            week_end: &end_key,
            days: &days,
            top_apps: &top_apps,
        };
        let system = build_weekly_system_prompt(&ai);
        let user_text = build_weekly_user_prompt(&ai, &ctx);

        let step2 = build_step2(&ai, port, ai.effective_summary_main())?;
        let step2_model = step2.model_label().to_string();

        // 推一条 summarizing 让前端段卡片切到"生成中"
        {
            let mut p = SummaryProgress::base(
                WEEKLY_SOURCE.to_string(),
                week_key.clone(),
                "summarizing",
                1,
            );
            p.segment_idx = Some(WEEKLY_SEGMENT_IDX);
            self.emit(p);
        }

        let _inflight = step2
            .is_local()
            .then(|| self.supervisor.acquire_inference());
        let (row, status_str): (SegmentSummaryRow, &'static str) =
            match step2.chat(&system, &user_text, &[]).await {
                Ok((content, _usage)) => (
                    SegmentSummaryRow {
                        source: WEEKLY_SOURCE.to_string(),
                        local_date: week_key.clone(),
                        segment_idx: WEEKLY_SEGMENT_IDX,
                        label: weekly_label(&week_key, &end_key),
                        start_hour: 0,
                        end_hour: 0,
                        content,
                        model: step2_model.clone(),
                        status: "ok".to_string(),
                        error: None,
                        generated_at: utc_now_rfc3339(),
                    },
                    "ok",
                ),
                Err(e) => (
                    SegmentSummaryRow {
                        source: WEEKLY_SOURCE.to_string(),
                        local_date: week_key.clone(),
                        segment_idx: WEEKLY_SEGMENT_IDX,
                        label: weekly_label(&week_key, &end_key),
                        start_hour: 0,
                        end_hour: 0,
                        content: String::new(),
                        model: step2_model,
                        status: "error".to_string(),
                        error: Some(e.to_string()),
                        generated_at: utc_now_rfc3339(),
                    },
                    "error",
                ),
            };

        // upsert 失败不让整轮抛飞——日报路径同款防御：磁盘满 / DB lock 时 row
        // 写不进去也得让上层 emit segment_done，至少前端能看到红色 error badge
        if let Err(e) = ai_summaries::upsert_segment(&self.pool, &row).await {
            log::error!(
                "ai_summaries upsert 失败（weekly {} status={}）：{e}",
                week_key,
                row.status
            );
        }

        // emit segment_done 把 row 推给前端
        let mut p = SummaryProgress::base(
            WEEKLY_SOURCE.to_string(),
            week_key.clone(),
            "segment_done",
            1,
        );
        p.segment_idx = Some(WEEKLY_SEGMENT_IDX);
        p.status = Some(status_str);
        p.content = Some(row.content.clone());
        p.message = row.error.clone();
        self.emit(p);

        self.emit_phase("all_done", &week_key, None, None);
        Ok(())
    }

    /// 这一周是否已有 ok 行（用于 force_refresh=false 早返回）。
    async fn weekly_already_ok(&self, week_key: &str) -> Result<bool> {
        let status = ai_summaries::get_segment_status(
            &self.pool,
            WEEKLY_SOURCE,
            week_key,
            WEEKLY_SEGMENT_IDX,
        )
        .await?;
        Ok(status.as_deref() == Some("ok"))
    }

    /// 启动引擎（如果还没起），返回端口。
    ///
    /// 跟 `DaySummaryRunner::ensure_engine_running` 同语义——但只关心 Step::Summary
    /// 因为周报全程纯文本，不需要 vision describe 的 mmproj。
    async fn ensure_engine_running(&self, ai: &crate::ai::config::AiConfig) -> Result<u16> {
        let st = self.supervisor.status().await;
        if st.state == EngineState::Running {
            if let Some(p) = st.port {
                return Ok(p);
            }
        }
        let main_name = ai.effective_summary_main();
        let mmproj_name = ai.effective_summary_mmproj();
        let models_dir = models::root_dir(ai);
        let main_path = models_dir.join(main_name);
        if !main_path.exists() {
            return Err(Error::ModelFileMissing(format!(
                "{}（可能被删除或路径变了）",
                main_name
            )));
        }
        let mmproj_path = if mmproj_name.trim().is_empty() {
            None
        } else {
            let p = models_dir.join(mmproj_name);
            if !p.exists() {
                return Err(Error::ModelFileMissing(format!(
                    "vision 投影 {}",
                    mmproj_name
                )));
            }
            Some(p)
        };
        let overrides = EngineStartOverrides {
            batch_size: ai.summary_batch_size_effective(),
            parallel_slots: ai.summary_parallel_slots_effective(),
            ctx_size: ai.summary_ctx_size_effective(),
        };
        self.supervisor
            .start_with_overrides(Some(main_path), mmproj_path, overrides)
            .await
    }

    fn emit(&self, payload: SummaryProgress) {
        if let Err(e) = self.app.emit(SUMMARY_PROGRESS_EVENT, &payload) {
            log::warn!("emit {SUMMARY_PROGRESS_EVENT} 失败: {e}");
        }
    }

    /// 简化版 emit——大多数 phase 字段都为 None，统一用 base 兜底。
    fn emit_phase(
        &self,
        phase: &'static str,
        week_key: &str,
        segment_idx: Option<u32>,
        message: Option<String>,
    ) {
        let mut p =
            SummaryProgress::base(WEEKLY_SOURCE.to_string(), week_key.to_string(), phase, 1);
        p.segment_idx = segment_idx;
        p.message = message;
        self.emit(p);
    }
}

/// 把一周内所有 daily 段总结按日期 group + 拼成日维度文本。
///
/// - 跳过 status != "ok" 的段（skipped / error 段没内容，进上下文也是噪声）
/// - 同一天内多段按 `segment_idx` 升序拼接，段间 `\n\n` 分隔
/// - 当天没任何 ok 段则该日**不**进结果——LLM 只看到有数据的天，逻辑更干净
/// - 返回顺序按日期升序
fn group_days(
    rows: &[SegmentSummaryRow],
    week_start: NaiveDate,
    week_end: NaiveDate,
    lang: &str,
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut day = week_start;
    while day <= week_end {
        let day_str = day.format("%Y-%m-%d").to_string();
        let mut segs: Vec<&SegmentSummaryRow> = rows
            .iter()
            .filter(|r| r.local_date == day_str && r.status == "ok" && !r.content.trim().is_empty())
            .collect();
        if !segs.is_empty() {
            segs.sort_by_key(|r| r.segment_idx);
            let mut day_text = String::new();
            for (i, r) in segs.iter().enumerate() {
                if i > 0 {
                    day_text.push_str("\n\n");
                }
                // 段头让 LLM 区分时段，不必再让它从语境推断
                day_text.push_str(&format!(
                    "[{} {:02}:00–{:02}:00] ",
                    r.label, r.start_hour, r.end_hour,
                ));
                day_text.push_str(r.content.trim());
            }
            let weekday = weekday_short(lang, day.weekday());
            out.push((day_str, weekday.to_string(), day_text));
        }
        day += Duration::days(1);
    }
    out
}

/// 周标签（"YYYY-MM-DD ~ YYYY-MM-DD"），写入 ai_summaries.label。
/// label 列在 daily 用作时段标签，weekly 这里复用作"本周日期范围"——前端展示时
/// 知道是 weekly 行就按"周"渲染，不会跟时段标签混。
fn weekly_label(week_start: &str, week_end: &str) -> String {
    format!("{} ~ {}", week_start, week_end)
}

/// 当某天没日报但有活动数据时，把当日 top apps 列表拼成一段"代日报"文本。
///
/// 拼装格式：第一行打 marker 标签让 LLM 一眼识别"这天没日报、只有应用统计"；
/// 余下是跟段总结 user prompt 同款的应用列表。三语都遵守同样的 marker 结构，
/// weekly_*.md 里有对应说明告诉模型遇到 marker 时怎么处理。
fn format_missing_day_fallback(lang: &str, day_apps: &[(String, u32, String)]) -> String {
    let (marker, header) = match lang {
        "en" => ("[No daily report; app stats only]", "Top apps used:"),
        "ja" => (
            "[この日は日報なし、アプリ統計のみ]",
            "最も使用されたアプリ：",
        ),
        _ => ("[当日无日报，仅应用统计]", "使用最多的应用："),
    };
    let mut out = String::new();
    out.push_str(marker);
    out.push('\n');
    out.push_str(header);
    out.push('\n');
    for (name, minutes, category) in day_apps.iter().take(8) {
        match lang {
            "en" => out.push_str(&format!("- {} ({} min · {})\n", name, minutes, category)),
            "ja" => out.push_str(&format!("- {}（{} 分 · {}）\n", name, minutes, category)),
            _ => out.push_str(&format!("- {}（{} 分钟 · {}）\n", name, minutes, category)),
        }
    }
    out
}

/// precheck 返回项——一天的元数据（前端弹框显示用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekPrecheckDay {
    /// "YYYY-MM-DD"
    pub date: String,
    /// 按当前 prompt 语言写的星期简写（"周一" / "Mon" / "月"）
    pub weekday: String,
    /// 该日是否有 daily ok 段总结（status='ok' 且 content 非空）
    pub has_daily: bool,
    /// 该日是否有 activity 记录（活动表非空 + 过滤掉 excluded_categories 后仍有内容）
    pub has_activity: bool,
}

/// precheck 命令的返回 payload——把一周 7 天拆成"有日报 / 仅活动 / 完全空"三档。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekPrecheckResp {
    /// 一周 7 天，按周一到周日顺序
    pub days: Vec<WeekPrecheckDay>,
    /// 7 天里有几天有 daily ok 段总结
    pub days_with_daily: u32,
    /// 7 天里有几天 has_activity = true 但 has_daily = false（前端"用活动统计替代"备选项）
    pub days_activity_only: u32,
}

/// 给前端"点击生成前预览"用的查询：拿这周 7 天的日报 / 活动覆盖情况。
///
/// 不依赖 [`WeekSummaryRunner`]，纯函数形式——前端可以单独 invoke 而无需触发任何
/// 引擎 / 模型加载，是"生成前确认"流程的支点。
pub async fn precheck_week(pool: &DbPool, week_start: NaiveDate) -> Result<WeekPrecheckResp> {
    let week_end = week_start + Duration::days(6);
    let week_key = week_start.format("%Y-%m-%d").to_string();
    let end_key = week_end.format("%Y-%m-%d").to_string();

    let cfg = settings_repo::load(pool).await?;
    let ai = cfg.ai.clone();
    let lang = ai.prompt_language.as_str().to_string();

    let daily_rows = ai_summaries::get_range(pool, "daily", &week_key, &end_key).await?;
    let present_daily: HashSet<String> = daily_rows
        .iter()
        .filter(|r| r.status == "ok" && !r.content.trim().is_empty())
        .map(|r| r.local_date.clone())
        .collect();

    let mut days_out: Vec<WeekPrecheckDay> = Vec::with_capacity(7);
    let mut days_with_daily: u32 = 0;
    let mut days_activity_only: u32 = 0;

    let mut day = week_start;
    while day <= week_end {
        let day_str = day.format("%Y-%m-%d").to_string();
        let has_daily = present_daily.contains(&day_str);

        // 查当日 top apps：哪怕只有 1 条 → has_activity = true
        // 失败不抛——单日 fallback 失败时按"没活动"处理，前端流程不卡
        let has_activity = if has_daily {
            // 有日报就视作有活动，跳过当日 top_apps 查询省时
            true
        } else {
            match ai_summaries::list_range_top_apps(
                pool,
                &day_str,
                &day_str,
                &ai.excluded_categories,
                DeviceFilter::All,
                1,
            )
            .await
            {
                Ok(rows) => !rows.is_empty(),
                Err(e) => {
                    log::warn!("precheck_week 查 top apps 失败（{}）：{e}", day_str);
                    false
                }
            }
        };

        if has_daily {
            days_with_daily += 1;
        } else if has_activity {
            days_activity_only += 1;
        }

        days_out.push(WeekPrecheckDay {
            date: day_str,
            weekday: weekday_short(&lang, day.weekday()).to_string(),
            has_daily,
            has_activity,
        });

        day += Duration::days(1);
    }

    Ok(WeekPrecheckResp {
        days: days_out,
        days_with_daily,
        days_activity_only,
    })
}
