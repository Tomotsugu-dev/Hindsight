//! AI 日报的编排核心（报告并轨后的单步管线）。
//!
//! [`DaySummaryRunner::run`] 是单点入口：拿到一天 + 设备过滤 + 是否强刷，
//! 内部按 settings.ai.segments 切段，串行跑每一段：
//!
//!   1. 从 activities 合成该段的逐小时活动时间线（应用时长 + 窗口标题样例）
//!   2. 拉段内 top apps 统计
//!   3. 拼 prompt + 调 LLM（本地 llama-server 或云端 API）
//!   4. 把段总结落 DB（status: ok / skipped_no_activity / error）
//!
//! 不再有 step 1（VLM 逐图描述）与 MobileNet 去重——窗口标题时间线是唯一材料源。
//! 串行而非并发：本地 llama-server 是单实例，并发请求只会让 llama.cpp 内部排队。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::NaiveDate;
use tauri::{AppHandle, Emitter};

use crate::ai::config::AiConfig;
use crate::ai::models;
use crate::ai::server::{EngineStartOverrides, EngineState, EngineSupervisor};
use crate::ai::summary_operations::{
    build_activity_timeline, build_step2, summarize_segment, upsert_skipped_no_activity,
};
use crate::ai::summary_overrides::AiOverrides;
use crate::ai::summary_progress::{SummaryProgress, SUMMARY_PROGRESS_EVENT};
use crate::error::{Error, Result};
use crate::repo::ai_summaries::{self, list_segment_top_apps};
use crate::repo::reports::DeviceFilter;
use crate::repo::settings as settings_repo;
use crate::storage::DbPool;

/// 单点入口：跑一天的 AI 总结。lib.rs 通过 `app.manage` 不直接管它，
/// 而是命令体里临时构造（生命周期跟随单次调用）。
pub struct DaySummaryRunner {
    pool: DbPool,
    supervisor: Arc<EngineSupervisor>,
    app: AppHandle,
    /// 取消信号：段边界检查 + 在途 LLM 请求和引擎加载也会被
    /// [`crate::ai::summary_operations::cancellable`] 每 250ms 轮询中断。
    cancel: Arc<AtomicBool>,
}

impl DaySummaryRunner {
    /// 由命令体临时构造（不进 Tauri State）：每次跑一次总结时新建一份。
    /// `cancel` 来自全局 `SummaryCancel` 单例，前端调 cancel_day_summary 时被设 true。
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

    /// 跑某一天的全部段。
    ///
    /// `force_refresh = true` 会先清空当天 ai_summaries 行，否则已有 ok 段直接复用。
    /// `source` = "daily" / "debug" — DailyTab 跟 DebugTab 写各自命名空间互不污染。
    pub async fn run(
        &self,
        source: &str,
        local_date: NaiveDate,
        device: DeviceFilter,
        force_refresh: bool,
        overrides: Option<AiOverrides>,
    ) -> Result<()> {
        // needs_restart：debug 路径（AiOverrides 显式带了 engine 覆盖）下才触发
        // 「跑前 stop+start with overrides，跑后再 stop」；daily 路径靠 settings.ai
        // 的 engine 字段，引擎按需 lazy spawn 不主动重启
        let needs_restart = overrides
            .as_ref()
            .map(|o| o.needs_engine_restart())
            .unwrap_or(false);

        let result = self
            .run_inner(
                source,
                local_date,
                device,
                force_refresh,
                overrides,
                needs_restart,
            )
            .await;

        // 调试用 override 跑完无条件 stop 引擎——保证下次正常日报跑会以默认参数
        // lazy start，不让调试值污染后续会话
        if needs_restart {
            let _ = self.supervisor.stop().await;
        }

        // 停止按钮中断在途请求 / 引擎加载时从深处抛 SummaryCancelled——不是失败，
        // 这里统一优雅收尾：emit cancelled 让前端复位，命令返回 Ok
        if matches!(result, Err(Error::SummaryCancelled)) {
            let p = SummaryProgress::base(
                source.to_string(),
                local_date.format("%Y-%m-%d").to_string(),
                "cancelled",
                0,
            );
            self.emit(p);
            return Ok(());
        }
        result
    }

    /// 给引擎启动 / 换模的 future 包一层取消轮询：停止按钮在 30-90s 模型加载
    /// 期间也能生效。中断后 stop() 收掉半启动的子进程。
    async fn engine_start_cancellable(
        &self,
        fut: impl std::future::Future<Output = Result<u16>>,
    ) -> Result<u16> {
        match crate::ai::summary_operations::cancellable(&self.cancel, fut).await {
            Err(Error::SummaryCancelled) => {
                let _ = self.supervisor.stop().await;
                Err(Error::SummaryCancelled)
            }
            r => r,
        }
    }

    async fn run_inner(
        &self,
        source: &str,
        local_date: NaiveDate,
        device: DeviceFilter,
        force_refresh: bool,
        overrides: Option<AiOverrides>,
        needs_restart: bool,
    ) -> Result<()> {
        let cfg = settings_repo::load(&self.pool).await?;
        // overrides 只对本次调用生效，不写回 settings
        let ai = match overrides {
            Some(o) => o.with_overrides(cfg.ai.clone()),
            None => cfg.ai.clone(),
        };

        if !ai.summary_use_cloud() && ai.effective_summary_main().trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」给段总结选一个模型，或选定云端 API 跑总结",
            ));
        }

        let date_str = local_date.format("%Y-%m-%d").to_string();
        if force_refresh {
            ai_summaries::clear_day(&self.pool, source, &date_str).await?;
        }

        let total_segments = ai.segments.len() as u32;

        // 调试 override 触发的强制重启：先 stop 把现役 llama-server 收掉
        if needs_restart {
            let _ = self.supervisor.stop().await;
        }

        // 单一引擎全程复用：云端不起本地引擎（0 为占位端口，External 分支不使用）
        let summary_overrides = EngineStartOverrides {
            batch_size: ai.summary_batch_size_effective(),
            parallel_slots: ai.summary_parallel_slots_effective(),
            ctx_size: ai.summary_ctx_size_effective(),
        };
        let port = self
            .ensure_summary_engine(source, &date_str, &ai, summary_overrides, total_segments)
            .await?;
        let step2 = build_step2(&ai, port, ai.effective_summary_main())?;

        for (idx, seg) in ai.segments.iter().enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                let mut p = SummaryProgress::base(
                    source.to_string(),
                    date_str.clone(),
                    "cancelled",
                    total_segments,
                );
                p.segment_idx = Some(idx as u32);
                self.emit(p);
                return Ok(());
            }
            if seg.end_hour <= seg.start_hour {
                continue;
            }
            // force_refresh=false 时已生成的段 (status=ok) 直接复用
            if !force_refresh
                && self
                    .segment_already_ok(source, &date_str, idx as u32)
                    .await?
            {
                continue;
            }
            self.run_one_segment(
                source,
                &step2,
                &ai,
                &date_str,
                idx as u32,
                total_segments,
                seg.label.clone(),
                seg.start_hour,
                seg.end_hour,
                device.clone(),
            )
            .await?;
        }

        let p = SummaryProgress::base(source.to_string(), date_str, "all_done", total_segments);
        self.emit(p);
        Ok(())
    }

    fn emit(&self, payload: SummaryProgress) {
        if let Err(e) = self.app.emit(SUMMARY_PROGRESS_EVENT, &payload) {
            log::warn!("emit {SUMMARY_PROGRESS_EVENT} 失败: {e}");
        }
    }

    /// 该段是否已生成且 status="ok"——给 force_refresh=false 跳过逻辑用。
    async fn segment_already_ok(
        &self,
        source: &str,
        date_str: &str,
        segment_idx: u32,
    ) -> Result<bool> {
        let status =
            ai_summaries::get_segment_status(&self.pool, source, date_str, segment_idx).await?;
        Ok(status.as_deref() == Some("ok"))
    }

    /// 云端总结跳过本地引擎；本地则确保引擎按 summary 模型 + 参数在跑，返回端口。
    async fn ensure_summary_engine(
        &self,
        source: &str,
        date_str: &str,
        ai: &AiConfig,
        engine_overrides: EngineStartOverrides,
        total_segments: u32,
    ) -> Result<u16> {
        if ai.summary_use_cloud() {
            log::info!("日报：段总结走云端，跳过本地引擎启动");
            return Ok(0);
        }
        let st = self.supervisor.status().await;
        if st.state != EngineState::Running {
            let mut p = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "engine_starting",
                total_segments,
            );
            // message 留空：前端按 phase 显示本地化的"加载模型中…"（dailySummary.ts）
            p.message = None;
            self.emit(p);
        }
        if st.state == EngineState::Running {
            if let Some(p) = st.port {
                let (main_path, _) = self.resolve_summary_model_paths(ai)?;
                // 模型 **和启动参数** 都匹配才复用；不匹配则重启换参
                if self.supervisor.loaded_main().as_deref() == Some(main_path.as_path())
                    && self.supervisor.loaded_overrides() == engine_overrides
                {
                    // 复用前"续命"：避免 idle watcher 在准备材料的几秒里杀掉 server
                    self.supervisor.touch();
                    return Ok(p);
                }
                log::info!("日报：已加载模型/参数与需求不符，重启换模");
                if let Err(e) = self.supervisor.stop().await {
                    log::warn!("换模前 stop 引擎失败（继续尝试启动）: {e}");
                }
            }
        }
        let (main_path, mmproj_path) = self.resolve_summary_model_paths(ai)?;
        self.engine_start_cancellable(self.supervisor.start_with_overrides(
            Some(main_path),
            mmproj_path,
            engine_overrides,
        ))
        .await
    }

    /// summary 模型的 GGUF 路径。文件不存在抛 `ModelFileMissing`。
    fn resolve_summary_model_paths(&self, ai: &AiConfig) -> Result<(PathBuf, Option<PathBuf>)> {
        let main_name = ai.effective_summary_main();
        let mmproj_name = ai.effective_summary_mmproj();
        let models_dir = models::root_dir(ai);
        let main_path: PathBuf = models_dir.join(main_name);
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
        Ok((main_path, mmproj_path))
    }

    /// 跑单段：活动时间线 + top_apps → LLM → 落 ai_summaries。
    ///
    /// 落库语义：
    /// - DB 写操作错误（IO 等）向上抛
    /// - LLM 调用失败：写一行 status='error'，不抛
    /// - 整段无活动：status='skipped_no_activity'
    #[allow(clippy::too_many_arguments)]
    async fn run_one_segment(
        &self,
        source: &str,
        step2: &crate::ai::llm::Step2Chat,
        ai: &AiConfig,
        date_str: &str,
        idx: u32,
        total_segments: u32,
        label: String,
        start_hour: u8,
        end_hour: u8,
        device: DeviceFilter,
    ) -> Result<()> {
        let step2_model = step2.model_label().to_string();

        let mut p_started = SummaryProgress::base(
            source.to_string(),
            date_str.to_string(),
            "segment_started",
            total_segments,
        );
        p_started.segment_idx = Some(idx);
        p_started.images_total = Some(0);
        self.emit(p_started);

        // 隐私关键词在 Settings 顶层，不在 AiConfig 里——每段一次 ad-hoc load，量级可忽略
        let cfg = settings_repo::load(&self.pool).await?;
        let timeline = build_activity_timeline(
            &self.pool,
            date_str,
            start_hour,
            end_hour,
            &ai.excluded_categories,
            &device,
            &cfg.privacy_app_keywords,
        )
        .await?;

        if timeline.is_empty() {
            // 真的什么都没有 —— skipped_no_activity 兜底
            upsert_skipped_no_activity(
                &self.pool,
                source,
                date_str,
                idx,
                &label,
                start_hour,
                end_hour,
                step2_model,
            )
            .await?;
            let mut p_done = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "segment_done",
                total_segments,
            );
            p_done.segment_idx = Some(idx);
            p_done.images_total = Some(0);
            p_done.content = Some(String::new());
            p_done.status = Some("skipped_no_activity");
            self.emit(p_done);
            return Ok(());
        }

        let top_apps = list_segment_top_apps(
            &self.pool,
            date_str,
            start_hour,
            end_hour,
            &ai.excluded_categories,
            device.clone(),
            8,
        )
        .await
        .unwrap_or_default();

        let mut p_sum = SummaryProgress::base(
            source.to_string(),
            date_str.to_string(),
            "summarizing",
            total_segments,
        );
        p_sum.segment_idx = Some(idx);
        p_sum.images_total = Some(0);
        self.emit(p_sum);

        let (row, status_str) = summarize_segment(
            &self.pool,
            step2,
            &self.supervisor,
            ai,
            source,
            date_str,
            &label,
            start_hour,
            end_hour,
            idx,
            &timeline,
            &top_apps,
            step2.model_label().to_string(),
            &self.cancel,
        )
        .await?;

        let mut p_done = SummaryProgress::base(
            source.to_string(),
            date_str.to_string(),
            "segment_done",
            total_segments,
        );
        p_done.segment_idx = Some(idx);
        p_done.images_total = Some(0);
        p_done.content = Some(row.content.clone());
        p_done.status = Some(status_str);
        p_done.message = row.error.clone();
        self.emit(p_done);
        Ok(())
    }

    /// "重试某段"专用：只跑指定一段，复用现有引擎。
    pub async fn run_one_segment_only(
        &self,
        source: &str,
        local_date: NaiveDate,
        segment_idx: u32,
        device: DeviceFilter,
        overrides: Option<AiOverrides>,
    ) -> Result<()> {
        let needs_restart = overrides
            .as_ref()
            .map(|o| o.needs_engine_restart())
            .unwrap_or(false);

        let result = self
            .run_one_segment_only_inner(
                source,
                local_date,
                segment_idx,
                device,
                overrides,
                needs_restart,
            )
            .await;
        if needs_restart {
            let _ = self.supervisor.stop().await;
        }
        result
    }

    async fn run_one_segment_only_inner(
        &self,
        source: &str,
        local_date: NaiveDate,
        segment_idx: u32,
        device: DeviceFilter,
        overrides: Option<AiOverrides>,
        needs_restart: bool,
    ) -> Result<()> {
        let cfg = settings_repo::load(&self.pool).await?;
        let ai = match overrides {
            Some(o) => o.with_overrides(cfg.ai.clone()),
            None => cfg.ai.clone(),
        };

        if !ai.summary_use_cloud() && ai.effective_summary_main().trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」给段总结选一个模型，或选定云端 API 跑总结",
            ));
        }

        let seg = ai
            .segments
            .get(segment_idx as usize)
            .cloned()
            .ok_or_else(|| Error::InvalidInputDyn(format!("段下标越界：{}", segment_idx)))?;
        if seg.end_hour <= seg.start_hour {
            return Err(Error::InvalidInput("段时间范围非法"));
        }

        let date_str = local_date.format("%Y-%m-%d").to_string();

        if needs_restart {
            let _ = self.supervisor.stop().await;
        }
        let summary_overrides = EngineStartOverrides {
            batch_size: ai.summary_batch_size_effective(),
            parallel_slots: ai.summary_parallel_slots_effective(),
            ctx_size: ai.summary_ctx_size_effective(),
        };
        let port = self
            .ensure_summary_engine(
                source,
                &date_str,
                &ai,
                summary_overrides,
                ai.segments.len() as u32,
            )
            .await?;
        let step2 = build_step2(&ai, port, ai.effective_summary_main())?;

        self.run_one_segment(
            source,
            &step2,
            &ai,
            &date_str,
            segment_idx,
            ai.segments.len() as u32,
            seg.label,
            seg.start_hour,
            seg.end_hour,
            device,
        )
        .await
    }
}
