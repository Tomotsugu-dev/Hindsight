//! AI 总结的编排核心（Phase 1B-γ）。
//!
//! [`DaySummaryRunner::run`] 是单点入口：拿到一天 + 设备过滤 + 是否强刷，
//! 内部按 settings.ai.segments 切段，串行跑每一段：
//!
//!   1. 拉段内截图 + top apps
//!   2. 截图等距下采样到 max_images_per_segment 张
//!   3. 每张转成 data URI（supports 并发）
//!   4. 拼 prompt + 调 ChatClient
//!   5. 把段总结落 DB（status: ok / skipped_no_screenshots / error）
//!
//! 串行而非并发：本地 llama-server 是单实例，并发请求只会让 llama.cpp 内部排队。
//! 但 step 1 单图描述支持 `parallel_slots > 1` 时并发跑（详见 `summary_operations`）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{NaiveDate, Utc};
use tauri::{AppHandle, Emitter};

use crate::ai::config::AiConfig;
use crate::ai::dedup;
use crate::ai::embedding;
use crate::ai::image::{pick_frames, to_data_uri};
use crate::ai::llm::ChatClient;
use crate::ai::models;
use crate::ai::prompt::{build_image_describe_system_prompt, build_image_describe_user_prompt};
use crate::ai::server::{EngineStartOverrides, EngineState, EngineSupervisor};
use crate::ai::summary_operations::{
    build_step2, describe_images, extract_time_label, summarize_segment, upsert_skipped_segment,
    SUMMARY_IMAGE_MAX_DIM,
};
use crate::ai::summary_overrides::AiOverrides;
use crate::ai::summary_progress::{SummaryProgress, SUMMARY_PROGRESS_EVENT};
use crate::error::{Error, Result};
use crate::repo::ai_summaries::{
    self, list_segment_screenshots, list_segment_top_apps, ImageDescriptionRow, ScreenshotMeta,
};
use crate::repo::embeddings as embeddings_repo;
use crate::repo::reports::DeviceFilter;
use crate::repo::settings as settings_repo;
use crate::storage::DbPool;

/// AI summary pipeline 里的两个阶段——决定加载哪份模型 + 用哪套 batch / -np / ctx。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// step 1：单图描述（多图并发跑）。模型走 `effective_describe_main`。
    Describe,
    /// step 2：段总结（基于 step 1 的描述拼上下文跑纯文本）。模型走 `effective_summary_main`。
    Summary,
}

/// 单点入口：跑一天的 AI 总结。lib.rs 通过 `app.manage` 不直接管它，
/// 而是命令体里临时构造（生命周期跟随单次调用）。
pub struct DaySummaryRunner {
    pool: DbPool,
    supervisor: Arc<EngineSupervisor>,
    app: AppHandle,
    /// 取消信号：每段开跑前检查；true 时整轮停止（落库写到了哪段就到哪段）。
    /// 不能中断已经在路上的 LLM 请求——一段 30-180s 的 chat 必须跑完才能 yield。
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
    /// `force_refresh = true` 会先清空当天 ai_summaries 行，否则已有的段直接复用
    /// （前端在调命令前应该已经判断过是否需要重跑）。
    /// `source` = "daily" / "debug" — DailyTab 跟 DebugTab 写各自命名空间互不污染。
    // 8 个参数对应 8 个调用方语义（日期 / 设备 / 强刷 / overrides / source 命名空间 /
    // step1_only / step2_only），拆 struct 反而把命令边界 schema 嵌一层
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        &self,
        source: &str,
        local_date: NaiveDate,
        device: DeviceFilter,
        force_refresh: bool,
        overrides: Option<AiOverrides>,
        // step1_only：调试 tab 「仅生成图片描述」触发，跳过 step 2（段总结）。
        // step2_only：调试 tab 「仅生成段总结」触发，跳过 step 1（直接读 DB 中已存的图描述）。
        // 互斥；都为 true 时 step1_only 优先（前端不应同时传，但兜底防御）。daily 路径都 false。
        step1_only: bool,
        step2_only: bool,
    ) -> Result<()> {
        // needs_restart：debug 路径（AiOverrides 显式带了 engine 覆盖）下才触发
        // 「跑前 stop+start with overrides，跑后再 stop」；daily 路径靠 settings.ai
        // 的 engine 字段，引擎按需 lazy spawn 不主动重启
        let needs_restart = overrides
            .as_ref()
            .map(|o| o.needs_engine_restart())
            .unwrap_or(false);

        let result = self
            .run_with_overrides_inner(
                source,
                local_date,
                device,
                force_refresh,
                overrides,
                needs_restart,
                step1_only,
                step2_only,
            )
            .await;

        // 调试用 override 跑完无条件 stop 引擎——保证下次正常日报跑会以默认参数
        // lazy start，不让调试值污染后续会话
        if needs_restart {
            let _ = self.supervisor.stop().await;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_with_overrides_inner(
        &self,
        source: &str,
        local_date: NaiveDate,
        device: DeviceFilter,
        force_refresh: bool,
        overrides: Option<AiOverrides>,
        needs_restart: bool,
        step1_only: bool,
        step2_only: bool,
    ) -> Result<()> {
        let cfg = settings_repo::load(&self.pool).await?;
        // overrides 只对本次调用生效，不写回 settings；settings 自己永远是用户在
        // AI 设置里配的"全局值"，调试 tab 改的本地参数从这里进来。
        // with_overrides 也会把 batch_size / parallel_slots / ctx_size 合并进 ai.*，
        // 所以下面统一从 ai.* 取 engine_overrides + parallel，无需分两条路。
        let ai = match overrides {
            Some(o) => o.with_overrides(cfg.ai.clone()),
            None => cfg.ai.clone(),
        };

        // 双套引擎参数：图描述阶段（slots 高、ctx 中）和段总结阶段（slots=1、ctx 高）
        // 各有自己的一组 EngineStartOverrides。新字段未设时通过 *_effective 自动 fallback
        // 到旧的全局 batch_size / parallel_slots / ctx_size，旧 settings JSON 不需要 migration。
        let describe_overrides = EngineStartOverrides {
            batch_size: ai.describe_batch_size_effective(),
            parallel_slots: ai.describe_parallel_slots_effective(),
            ctx_size: ai.describe_ctx_size_effective(),
        };
        let summary_overrides = EngineStartOverrides {
            batch_size: ai.summary_batch_size_effective(),
            parallel_slots: ai.summary_parallel_slots_effective(),
            ctx_size: ai.summary_ctx_size_effective(),
        };
        // step 1 图描述并发数 = describe 阶段的 -np（多图同时跑）
        let parallel = ai.describe_parallel_slots_effective().unwrap_or(1).max(1) as usize;

        // step 1 必须有本地 vision 模型；step 2 要么有本地模型，要么 external_enabled 走云端
        if ai.effective_describe_main().trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」给图描述选一个 vision 模型再生成总结",
            ));
        }
        if !ai.external_enabled && ai.effective_summary_main().trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」给段总结选一个模型，或在「云端 API」启用云端总结",
            ));
        }

        let date_str = local_date.format("%Y-%m-%d").to_string();
        if force_refresh {
            // step2_only：只能清段总结，逐图描述要保留给 step 2 用——否则 force_refresh
            // 一刀切把 step 1 数据也删了，run_one_segment_step2_only 拿到空数据全段 skipped。
            if step2_only {
                ai_summaries::clear_day_summaries_only(&self.pool, source, &date_str).await?;
            } else {
                ai_summaries::clear_day(&self.pool, source, &date_str).await?;
            }
        }

        let total_segments = ai.segments.len() as u32;

        // 调试 override 触发的强制重启：先 stop 把现役 llama-server 收掉，
        // 再 ensure_engine_running 走启动分支，把 batch / -np 灌进去
        if needs_restart {
            let _ = self.supervisor.stop().await;
        }

        // 决定执行计划：
        //   step1_only=true  → 调试 tab「仅生成图片描述」：所有段批量跑 step 1，phased
        //   step2_only=true  → 调试 tab「仅生成段总结」：所有段从 DB 读描述跑 step 2，phased
        //   都 false（daily 路径） → **段间流水线**：每段 step 1 跑完立刻跑 step 2 →
        //                            前端立刻看到该段总结，不用等满整轮 phase 1。
        //                            代价：step 1/2 用不同模型时每段两次 server restart。

        let max_images = ai.max_images_per_segment as usize;
        // Daily 路径统一走 swap_per_segment：每段 step1→step2 中间 stop+start 切参数。
        // 无论 main/mmproj 是否一致，describe / summary 各自的 ctx × parallel 配置都会被
        // 独立采纳，不再合并 overrides——同 main 时 merge_max 会把 ctx 和 slots 都拉到最坏
        // 组合（如 8K×4 + 64K×1 → 64K×4），KV cache 直接爆 GPU/统一内存。
        // 代价：每段两次 model reload；同模型时也是同一文件 unload/reload，不可避免。

        if step1_only {
            // —— 调试 tab：只跑全段 step 1 ——
            self.run_phase_step1_all_segments(
                source,
                &date_str,
                &ai,
                describe_overrides.clone(),
                force_refresh,
                total_segments,
                max_images,
                device.clone(),
                parallel,
            )
            .await?;
        } else if step2_only {
            // —— 调试 tab：只跑全段 step 2（从 DB 读已存描述）——
            self.run_phase_step2_all_segments(
                source,
                &date_str,
                &ai,
                summary_overrides.clone(),
                force_refresh,
                total_segments,
                max_images,
                device.clone(),
                parallel,
            )
            .await?;
        } else {
            // —— Daily 流水线：每段 swap 两次 server，分别套用 describe / summary 参数。
            // 慢但 UX 好：每段 step 2 跑完前端立刻看到该段总结；且每个 phase 的 KV cache
            // 严格按当前 phase 配置预分配，不会因合并参数 OOM。
            log::info!("[engine-plan] interleaved_swap_per_segment: 每段切 describe/summary 参数");
            self.run_interleaved_swap_per_segment(
                source,
                &date_str,
                &ai,
                describe_overrides.clone(),
                summary_overrides.clone(),
                force_refresh,
                total_segments,
                max_images,
                device.clone(),
                parallel,
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

    /// 调试 tab「仅生成图片描述」：所有段批量跑 step 1，phased。
    ///
    /// 单一 server 加载 describe 模型，循环每段调 `run_one_segment(step1_only=true)`。
    /// 每段 emit `phase="step1_done"`（不写 ai_summaries），成败由该段内部 step 1 全失败兜底处理。
    #[allow(clippy::too_many_arguments)]
    async fn run_phase_step1_all_segments(
        &self,
        source: &str,
        date_str: &str,
        ai: &AiConfig,
        describe_overrides: EngineStartOverrides,
        force_refresh: bool,
        total_segments: u32,
        max_images: usize,
        device: DeviceFilter,
        parallel: usize,
    ) -> Result<()> {
        let st = self.supervisor.status().await;
        if st.state != EngineState::Running {
            let mut p = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "engine_starting",
                total_segments,
            );
            p.message = Some("加载模型中（首次约 30-90 秒）…".to_string());
            self.emit(p);
        }
        let port = self
            .ensure_engine_running(ai, Step::Describe, describe_overrides)
            .await?;
        let step1 = ChatClient::new(
            port,
            ai.effective_describe_main().to_string(),
            ai.describe_max_tokens(),
        )?;
        // step2 client 在 step1_only 路径不会被调用——构造一份占位的让 run_one_segment 签名能通过
        let step2_placeholder = build_step2(ai, port, ai.effective_describe_main())?;
        for (idx, seg) in ai.segments.iter().enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                let mut p = SummaryProgress::base(
                    source.to_string(),
                    date_str.to_string(),
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
            // C1：force_refresh=false 时已生成的段 (status=ok) 跳过——之前注释承诺
            // "已有的段直接复用"但代码没实现，导致每次跑都重写 ok 段
            if !force_refresh
                && self
                    .segment_already_ok(source, date_str, idx as u32)
                    .await?
            {
                continue;
            }
            self.run_one_segment(
                source,
                &step1,
                &step2_placeholder,
                ai,
                date_str,
                idx as u32,
                total_segments,
                seg.label.clone(),
                seg.start_hour,
                seg.end_hour,
                max_images,
                device.clone(),
                parallel,
                /* step1_only */ true,
                /* step2_only */ false,
            )
            .await?;
        }
        Ok(())
    }

    /// 调试 tab「仅生成段总结」：所有段从 DB 读已存的图描述跑 step 2，phased。
    /// 用 summary_overrides + summary 模型启动单一 server。
    #[allow(clippy::too_many_arguments)]
    async fn run_phase_step2_all_segments(
        &self,
        source: &str,
        date_str: &str,
        ai: &AiConfig,
        summary_overrides: EngineStartOverrides,
        force_refresh: bool,
        total_segments: u32,
        max_images: usize,
        device: DeviceFilter,
        parallel: usize,
    ) -> Result<()> {
        let st = self.supervisor.status().await;
        if st.state != EngineState::Running {
            let mut p = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "engine_starting",
                total_segments,
            );
            p.message = Some("加载模型中（首次约 30-90 秒）…".to_string());
            self.emit(p);
        }
        let port = self
            .ensure_engine_running(ai, Step::Summary, summary_overrides)
            .await?;
        let step1_placeholder = ChatClient::new(
            port,
            ai.effective_summary_main().to_string(),
            ai.summary_max_tokens(),
        )?;
        let step2 = build_step2(ai, port, ai.effective_summary_main())?;
        for (idx, seg) in ai.segments.iter().enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                let mut p = SummaryProgress::base(
                    source.to_string(),
                    date_str.to_string(),
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
            if !force_refresh
                && self
                    .segment_already_ok(source, date_str, idx as u32)
                    .await?
            {
                continue;
            }
            self.run_one_segment(
                source,
                &step1_placeholder,
                &step2,
                ai,
                date_str,
                idx as u32,
                total_segments,
                seg.label.clone(),
                seg.start_hour,
                seg.end_hour,
                max_images,
                device.clone(),
                parallel,
                /* step1_only */ false,
                /* step2_only */ true,
            )
            .await?;
        }
        Ok(())
    }

    /// 该段是否已生成且 status="ok"——给 force_refresh=false 跳过逻辑用。
    /// 任意非 ok 状态（None / "error" / "skipped_no_screenshots"）都返 false 让段重跑。
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

    /// Daily 流水线 —— 段间双 swap 模式（唯一 daily 路径）：每段 step1→step2 中间
    /// stop+start 切 describe / summary 各自的引擎参数（ctx / parallel / batch）：
    ///   段 0 step 1（describe params）→ restart → 段 0 step 2（summary params）→ restart →
    ///   段 1 step 1 → ...
    ///
    /// 即便 main / mmproj 完全相同也走这条：因为合并 overrides 会把 ctx 和 slots 同时
    /// 拉到最坏组合（如 8K×4 + 64K×1 → 64K×4），KV cache 直接吃爆 GPU/统一内存。
    /// 慢（每段两次 model load × N 段），但 UX 立竿见影：每段 step 2 跑完前端立刻看到
    /// 段总结。本地 GGUF 加载 30-90s × N 段 × 2 = 5-15 分钟额外开销，用户接受。
    #[allow(clippy::too_many_arguments)]
    async fn run_interleaved_swap_per_segment(
        &self,
        source: &str,
        date_str: &str,
        ai: &AiConfig,
        describe_overrides: EngineStartOverrides,
        summary_overrides: EngineStartOverrides,
        force_refresh: bool,
        total_segments: u32,
        max_images: usize,
        device: DeviceFilter,
        parallel: usize,
    ) -> Result<()> {
        for (idx, seg) in ai.segments.iter().enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                let mut p = SummaryProgress::base(
                    source.to_string(),
                    date_str.to_string(),
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
            if !force_refresh
                && self
                    .segment_already_ok(source, date_str, idx as u32)
                    .await?
            {
                continue;
            }

            // ── step 1：load describe model ──
            // 每段都 emit engine_starting 让前端知道在 swap（5-15 分钟级总等待）
            let mut p_load_d = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "engine_starting",
                total_segments,
            );
            p_load_d.segment_idx = Some(idx as u32);
            p_load_d.message = Some("加载图描述模型中…".to_string());
            self.emit(p_load_d);
            let (main_d, mmproj_d) = self.resolve_model_paths_for(ai, Step::Describe)?;
            let port_d = self
                .supervisor
                .restart_with_overrides(Some(main_d), mmproj_d, describe_overrides.clone())
                .await?;
            let step1 = ChatClient::new(
                port_d,
                ai.effective_describe_main().to_string(),
                ai.describe_max_tokens(),
            )?;
            // step2 占位——不会调用（step1_only=true）
            let step2_placeholder = build_step2(ai, port_d, ai.effective_describe_main())?;
            self.run_one_segment(
                source,
                &step1,
                &step2_placeholder,
                ai,
                date_str,
                idx as u32,
                total_segments,
                seg.label.clone(),
                seg.start_hour,
                seg.end_hour,
                max_images,
                device.clone(),
                parallel,
                /* step1_only */ true,
                /* step2_only */ false,
            )
            .await?;

            // step 1 跑完检查取消：避免白白 swap 加载 summary 模型
            if self.cancel.load(Ordering::Relaxed) {
                let mut p = SummaryProgress::base(
                    source.to_string(),
                    date_str.to_string(),
                    "cancelled",
                    total_segments,
                );
                p.segment_idx = Some(idx as u32);
                self.emit(p);
                return Ok(());
            }

            // A1：step 1 全失败兜底已经写了 ai_summaries error 行 + emit segment_done error。
            // 这种情况下没必要继续 swap 加载 summary 模型再让 step2_only 看到空 stored 重 emit 一遍——
            // 直接进下一段。注：metas 真空情况 run_one_segment 里走 skipped 路径写的是
            // status="skipped_no_screenshots"，也跳过没有意义（一致返还）。
            let seg_status =
                ai_summaries::get_segment_status(&self.pool, source, date_str, idx as u32).await?;
            if matches!(
                seg_status.as_deref(),
                Some("error" | "skipped_no_screenshots")
            ) {
                log::info!(
                    "swap_per_segment: 段 {idx} step 1 后 status={:?}，跳过 step 2 swap",
                    seg_status
                );
                continue;
            }

            // ── step 2：swap to summary model ──
            let mut p_load_s = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "engine_starting",
                total_segments,
            );
            p_load_s.segment_idx = Some(idx as u32);
            p_load_s.message = Some("加载段总结模型中…".to_string());
            self.emit(p_load_s);
            let (main_s, mmproj_s) = self.resolve_model_paths_for(ai, Step::Summary)?;
            let port_s = self
                .supervisor
                .restart_with_overrides(Some(main_s), mmproj_s, summary_overrides.clone())
                .await?;
            let step1_placeholder = ChatClient::new(
                port_s,
                ai.effective_summary_main().to_string(),
                ai.summary_max_tokens(),
            )?;
            let step2 = build_step2(ai, port_s, ai.effective_summary_main())?;
            self.run_one_segment(
                source,
                &step1_placeholder,
                &step2,
                ai,
                date_str,
                idx as u32,
                total_segments,
                seg.label.clone(),
                seg.start_hour,
                seg.end_hour,
                max_images,
                device.clone(),
                parallel,
                /* step1_only */ false,
                /* step2_only */ true,
            )
            .await?;
        }
        Ok(())
    }

    /// 跑单段——两步生成：
    ///   step 1：每张抽帧后的截图独立调 vision LLM 拿到一段描述 → 落 ai_image_descriptions
    ///   step 2：把所有描述 + top_apps 拼成纯文本，再调 LLM 写段总结 → 落 ai_summaries
    ///
    /// 落库语义：
    /// - DB 写操作错误（IO 等）向上抛
    /// - 单张图描述失败：log 跳过，不阻塞整段（多张图缺一两张可以接受）
    /// - 段总结调用失败：写一行 ai_summaries 行 status='error'，不抛
    #[allow(clippy::too_many_arguments)]
    async fn run_one_segment(
        &self,
        source: &str,
        step1: &ChatClient,
        step2: &crate::ai::llm::Step2Chat,
        ai: &AiConfig,
        date_str: &str,
        idx: u32,
        total_segments: u32,
        label: String,
        start_hour: u8,
        end_hour: u8,
        max_images: usize,
        device: DeviceFilter,
        parallel: usize,
        // step1_only=true 时跳过段总结那一步——前端调试 tab「仅生成图片描述」按钮触发，
        // 用户单独看 step 1 输出效果时不浪费时间在 step 2 上。
        step1_only: bool,
        // step2_only=true 时跳过逐图描述：从 DB 读出已存的 image descriptions 直接喂给 step 2。
        // 前端调试 tab「仅生成段总结」按钮触发；该段没存 step 1 数据时按 skipped 兜底。
        step2_only: bool,
    ) -> Result<()> {
        if step2_only {
            return self
                .run_one_segment_step2_only(
                    source,
                    step2,
                    ai,
                    date_str,
                    idx,
                    total_segments,
                    label,
                    start_hour,
                    end_hour,
                    device,
                )
                .await;
        }

        // step 1 落库的 model 是 describe 阶段实际加载的 GGUF 文件名；step 2 落库的 model
        // 由 step2.model_label() 给出（本地 = effective_summary_main，外部 = 用户填的 ID）
        let model = ai.effective_describe_main().to_string();
        let step2_model = step2.model_label().to_string();

        // ────── 取数据 ──────
        let metas = list_segment_screenshots(
            &self.pool,
            date_str,
            start_hour,
            end_hour,
            &ai.excluded_categories,
            device.clone(),
        )
        .await?;

        // 没截图的段直接 skipped 兜底
        if metas.is_empty() {
            let mut p_started = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "segment_started",
                total_segments,
            );
            p_started.segment_idx = Some(idx);
            p_started.images_total = Some(0);
            self.emit(p_started);

            upsert_skipped_segment(
                &self.pool, source, date_str, idx, &label, start_hour, end_hour, model,
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
            p_done.status = Some("skipped_no_screenshots");
            self.emit(p_done);
            return Ok(());
        }

        // ────── 相似度去重（Phase 1C） ──────
        // pick_frames 之前先按 MobileNet embedding 余弦阈值砍冗余画面：
        // 实测 0.95 阈值 ~70% 去重率，step 1 vision LLM 工作量直接降 3 倍。
        // 段间天然隔离（caller 是按段切的循环），不需要时间窗参数。
        // emit 一条 hint 让前端显示 "embedding 去重中…"——首次跑大段需要 N×5ms，
        // 一两千张 5-10s 不算瞬时
        let mut p_dedup = SummaryProgress::base(
            source.to_string(),
            date_str.to_string(),
            "dedup_running",
            total_segments,
        );
        p_dedup.segment_idx = Some(idx);
        p_dedup.images_total = Some(metas.len() as u32);
        self.emit(p_dedup);
        let metas = self.dedup_segment_metas(metas, ai.dedup_threshold).await?;

        // 等距抽帧
        let picked: Vec<ScreenshotMeta> = pick_frames(metas, max_images);

        // 段重跑前先清掉旧的逐图描述，避免新旧 image_index 错位
        ai_summaries::clear_segment_descriptions(&self.pool, source, date_str, idx).await?;

        let mut p_started = SummaryProgress::base(
            source.to_string(),
            date_str.to_string(),
            "segment_started",
            total_segments,
        );
        p_started.segment_idx = Some(idx);
        p_started.images_total = Some(picked.len() as u32);
        self.emit(p_started);

        let top_apps = list_segment_top_apps(
            &self.pool,
            date_str,
            start_hour,
            end_hour,
            &ai.excluded_categories,
            device,
            8,
        )
        .await
        .unwrap_or_default();

        // ────── step 1：逐图描述 ──────
        // parallel == 1：串行，跟历史行为一致；> 1：用 buffer_unordered 并发
        // N 路 chat，配合 llama-server `-np N` 才能真正吃到并行算力。
        // 结果按 image_index 排序（buffer_unordered 完成顺序不固定）后传给 step 2。
        // user prompt 在 describe_images 内 per-image 现拼，每张图带上自己的应用名 / 分类。
        let describe_system = build_image_describe_system_prompt(ai);

        let descriptions = describe_images(
            self.pool.clone(),
            self.app.clone(),
            Arc::clone(&self.supervisor),
            Arc::clone(&self.cancel),
            step1.clone(),
            &picked,
            ai.prompt_language.clone(),
            describe_system,
            model,
            date_str.to_string(),
            source.to_string(),
            idx,
            total_segments,
            parallel.max(1),
        )
        .await?;

        // 整段开始前已检查过 cancel；step 1 跑完后再查一次，避免 step 2 白跑
        if self.cancel.load(Ordering::Relaxed) {
            let mut p_cancel = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "cancelled",
                total_segments,
            );
            p_cancel.segment_idx = Some(idx);
            self.emit(p_cancel);
            return Ok(());
        }

        // step 1 全失败兜底：本段有 N 张待分析的图但一条描述都没拿到（典型场景：
        // llama-server 没起来 / 端口连不上，每张图都在 describe_images 里 silent skip）。
        // 必须放在 step1_only 早 return 之前——daily Phase 1 走的就是 step1_only=true，
        // 不挡在这里 Phase 2 会看到空 stored 然后凑空总结假成功。
        if !picked.is_empty() && descriptions.is_empty() {
            let err_msg = format!(
                "step 1 全失败：{} 张图描述都没拿到（多半是 llama-server 没起来 / 端口拒绝；调试 tab → 引擎日志 排查）",
                picked.len()
            );
            log::warn!("段 {idx} {err_msg}");
            let now = Utc::now().to_rfc3339();
            let row = crate::repo::ai_summaries::SegmentSummaryRow {
                source: source.to_string(),
                local_date: date_str.to_string(),
                segment_idx: idx,
                label: label.clone(),
                start_hour,
                end_hour,
                content: String::new(),
                model: ai.effective_describe_main().to_string(),
                status: "error".to_string(),
                error: Some(err_msg.clone()),
                generated_at: now,
            };
            ai_summaries::upsert_segment(&self.pool, &row).await?;
            let mut p_done = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "segment_done",
                total_segments,
            );
            p_done.segment_idx = Some(idx);
            p_done.images_total = Some(picked.len() as u32);
            p_done.content = Some(String::new());
            p_done.status = Some("error");
            p_done.message = Some(err_msg);
            self.emit(p_done);
            return Ok(());
        }

        // step1_only：跳过 step 2 的段总结。emit 一条 step1_done（**不是** segment_done）
        // 让前端知道 step 1 完了，但**不**把 row badge 切到"已生成"——只有 Phase 2 真跑完
        // 段总结后才发 segment_done 把 row 写进 ai_summaries。
        // 不写 ai_summaries 行：避免后续 daily Phase 2 期待"重新跑段总结"时被旧空 row 挡住。
        if step1_only {
            let mut p_done = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "step1_done",
                total_segments,
            );
            p_done.segment_idx = Some(idx);
            p_done.images_total = Some(picked.len() as u32);
            self.emit(p_done);
            return Ok(());
        }

        // ────── step 2：段总结（纯文本调用，不再传图） ──────
        // emit `summarizing` 让前端段卡片把 body 文案从"正在让模型看截图"切到"生成段总结中"。
        // step 2 是单次 chat 没有进度条，前端只显示 spinner + "生成段总结中…" 直到 segment_done。
        let mut p_sum = SummaryProgress::base(
            source.to_string(),
            date_str.to_string(),
            "summarizing",
            total_segments,
        );
        p_sum.segment_idx = Some(idx);
        p_sum.images_total = Some(picked.len() as u32);
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
            &descriptions,
            &top_apps,
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
        p_done.images_total = Some(picked.len() as u32);
        p_done.content = Some(row.content.clone());
        p_done.status = Some(status_str);
        p_done.message = row.error.clone();
        self.emit(p_done);
        Ok(())
    }

    /// step2-only 路径：跳过逐图描述，从 DB 读出已存的 image descriptions 直接喂 step 2。
    ///
    /// 空 stored 处理：在 daily 双 phase 流程里，Phase 1 已经为本段写过 ai_summaries 行
    /// （metas 真空 → skipped；step 1 全失败 → error），这里**不**重写表，只 emit
    /// segment_done 携带现有行的 status，让前端拿到一致状态。
    #[allow(clippy::too_many_arguments)]
    async fn run_one_segment_step2_only(
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
        // step2_only 路径里 step 1 描述早就落过库，model 字段只用于 skipped 兜底行
        let model = ai.effective_describe_main().to_string();
        let step2_model = step2.model_label().to_string();

        let stored =
            ai_summaries::get_segment_image_descriptions(&self.pool, source, date_str, idx).await?;

        // **不** emit segment_started——swap_per_segment 流程里上一步 step 1 已经发过
        // segment_started + image_described × N + step1_done。如果这里再 emit segment_started
        // 会让前端 runningStage 从 "summarizing" 重置到 "describing 0/0"，UI 闪一下。
        // debug step2_only 单跑时 summarizing emit 会自带 runningIdx，前端段卡片仍能切 running。

        if stored.is_empty() {
            // Phase 1 已为本段写好 row（真空 → skipped；step 1 全失败 → error）→ 读现有
            // status 反映给前端，不重写表保留 Phase 1 真相。现有行缺失（用户单跑 step2_only
            // 但 Phase 1 没跑过）→ 写一条 error 行提示。单次查询 + Option match，避免双查。
            let existing =
                ai_summaries::get_segment_status(&self.pool, source, date_str, idx).await?;
            let status_static: &'static str = match existing.as_deref() {
                Some("ok") => "ok",
                Some("skipped_no_screenshots") => "skipped_no_screenshots",
                Some(_) => "error",
                None => {
                    // 没行——写一条提示 error 让前端看到
                    let err_msg =
                        "段总结失败：找不到逐图描述（先跑一次 step 1，或检查日期是否对得上）"
                            .to_string();
                    let row = crate::repo::ai_summaries::SegmentSummaryRow {
                        source: source.to_string(),
                        local_date: date_str.to_string(),
                        segment_idx: idx,
                        label: label.clone(),
                        start_hour,
                        end_hour,
                        content: String::new(),
                        model,
                        status: "error".to_string(),
                        error: Some(err_msg),
                        generated_at: Utc::now().to_rfc3339(),
                    };
                    ai_summaries::upsert_segment(&self.pool, &row).await?;
                    "error"
                }
            };
            let mut p_done = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "segment_done",
                total_segments,
            );
            p_done.segment_idx = Some(idx);
            p_done.images_total = Some(0);
            p_done.content = Some(String::new());
            p_done.status = Some(status_static);
            self.emit(p_done);
            return Ok(());
        }

        if self.cancel.load(Ordering::Relaxed) {
            let mut p_cancel = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "cancelled",
                total_segments,
            );
            p_cancel.segment_idx = Some(idx);
            self.emit(p_cancel);
            return Ok(());
        }

        // ImageDescriptionRow → (time_label, description)：跟 step1 路径喂给 summarize_segment
        // 的格式对齐（time_label 从截图文件名解析 HH:MM）
        let descriptions: Vec<(String, String)> = stored
            .into_iter()
            .map(|r| (extract_time_label(&r.screenshot_path), r.description))
            .collect();

        let top_apps = list_segment_top_apps(
            &self.pool,
            date_str,
            start_hour,
            end_hour,
            &ai.excluded_categories,
            device,
            8,
        )
        .await
        .unwrap_or_default();

        // 同 run_one_segment：emit summarizing 让前端切到"生成段总结中"文案
        let mut p_sum = SummaryProgress::base(
            source.to_string(),
            date_str.to_string(),
            "summarizing",
            total_segments,
        );
        p_sum.segment_idx = Some(idx);
        p_sum.images_total = Some(descriptions.len() as u32);
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
            &descriptions,
            &top_apps,
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
        p_done.images_total = Some(descriptions.len() as u32);
        p_done.content = Some(row.content.clone());
        p_done.status = Some(status_str);
        p_done.message = row.error.clone();
        self.emit(p_done);
        Ok(())
    }

    /// 重新生成单张图的描述——调试 tab 的"重跑"按钮用。
    ///
    /// 不动段总结、其它图描述；只重写 ai_image_descriptions 一行。
    /// emit `image_described` 让前端实时刷新该行 UI。
    pub async fn retry_one_image_description(
        &self,
        source: &str,
        local_date: NaiveDate,
        segment_idx: u32,
        image_index: u32,
        overrides: Option<AiOverrides>,
    ) -> Result<()> {
        let needs_restart = overrides
            .as_ref()
            .map(|o| o.needs_engine_restart())
            .unwrap_or(false);
        let result = self
            .retry_one_image_inner(
                source,
                local_date,
                segment_idx,
                image_index,
                overrides,
                needs_restart,
            )
            .await;
        if needs_restart {
            let _ = self.supervisor.stop().await;
        }
        result
    }

    async fn retry_one_image_inner(
        &self,
        source: &str,
        local_date: NaiveDate,
        segment_idx: u32,
        image_index: u32,
        overrides: Option<AiOverrides>,
        needs_restart: bool,
    ) -> Result<()> {
        let cfg = settings_repo::load(&self.pool).await?;
        let ai = match overrides {
            Some(o) => o.with_overrides(cfg.ai.clone()),
            None => cfg.ai.clone(),
        };
        // engine_overrides 同 run() 一样从合并后的 ai.* 取，daily / debug 共用
        let engine_overrides = EngineStartOverrides {
            batch_size: ai.batch_size,
            parallel_slots: ai.parallel_slots,
            ctx_size: ai.ctx_size,
        };

        if ai.effective_describe_main().trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」给图描述选一个 vision 模型再调试",
            ));
        }

        let date_str = local_date.format("%Y-%m-%d").to_string();
        let total_segments = ai.segments.len() as u32;

        // 拉该段该 image_index 的现有行（拿 screenshot_path）
        let existing = ai_summaries::get_segment_image_descriptions(
            &self.pool,
            source,
            &date_str,
            segment_idx,
        )
        .await?;
        let existing_row = existing
            .into_iter()
            .find(|r| r.image_index == image_index)
            .ok_or_else(|| {
                Error::InvalidInputDyn(format!(
                    "段 {} 第 {} 张图没有现有描述，先跑一次完整生成",
                    segment_idx, image_index
                ))
            })?;

        if needs_restart {
            let _ = self.supervisor.stop().await;
        }
        // 启引擎（如未启动）——单图调试只走 step 1，加载 describe 模型
        let port = self
            .ensure_engine_running(&ai, Step::Describe, engine_overrides)
            .await?;
        let chat = ChatClient::new(
            port,
            ai.effective_describe_main().to_string(),
            ai.describe_max_tokens(),
        )?;

        // 反查这张图的 app/category 元数据；查不到就兜底用 path 当 app 名继续跑
        // —— 不能因为元数据丢失阻塞重跑，prompt 即使没分类也能正常工作。
        let meta = ai_summaries::get_screenshot_meta(&self.pool, &existing_row.screenshot_path)
            .await?
            .unwrap_or_else(|| ScreenshotMeta {
                path: existing_row.screenshot_path.clone(),
                app_display: existing_row.screenshot_path.clone(),
                category_name: None,
            });

        let describe_system = build_image_describe_system_prompt(&ai);
        let describe_user = build_image_describe_user_prompt(
            &ai.prompt_language,
            &meta.app_display,
            meta.category_name.as_deref(),
        );

        let data_uri = to_data_uri(
            Path::new(&existing_row.screenshot_path),
            SUMMARY_IMAGE_MAX_DIM,
        )
        .await?;

        let _inflight = self.supervisor.acquire_inference();
        let (desc, usage) = chat
            .chat_with_images(
                &describe_system,
                &describe_user,
                std::slice::from_ref(&data_uri),
            )
            .await?;
        drop(_inflight);

        // 覆盖落库
        ai_summaries::upsert_image_description(
            &self.pool,
            &ImageDescriptionRow {
                source: source.to_string(),
                local_date: date_str.clone(),
                segment_idx,
                image_index,
                screenshot_path: existing_row.screenshot_path.clone(),
                description: desc.clone(),
                model: ai.effective_describe_main().to_string(),
                generated_at: Utc::now().to_rfc3339(),
                latency_ms: Some(usage.latency_ms),
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
            },
        )
        .await?;

        // emit 让前端刷新
        let mut p = SummaryProgress::base(
            source.to_string(),
            date_str,
            "image_described",
            total_segments,
        );
        p.segment_idx = Some(segment_idx);
        p.image_index = Some(image_index);
        p.image_path = Some(existing_row.screenshot_path);
        p.image_description = Some(desc);
        p.latency_ms = Some(usage.latency_ms);
        p.prompt_tokens = usage.prompt_tokens;
        p.completion_tokens = usage.completion_tokens;
        self.emit(p);
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

    #[allow(clippy::too_many_arguments)]
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
        let engine_overrides = EngineStartOverrides {
            batch_size: ai.batch_size,
            parallel_slots: ai.parallel_slots,
            ctx_size: ai.ctx_size,
        };
        let parallel = ai.parallel_slots.unwrap_or(1).max(1) as usize;

        if ai.effective_describe_main().trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」给图描述选一个 vision 模型再生成总结",
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
        let st = self.supervisor.status().await;
        if st.state != EngineState::Running {
            let mut p = SummaryProgress::base(
                source.to_string(),
                date_str.clone(),
                "engine_starting",
                ai.segments.len() as u32,
            );
            p.message = Some("加载模型中（首次约 30-90 秒）…".to_string());
            self.emit(p);
        }
        // 单段重试只起一个引擎实例；如果用户给 step1/step2 配了不同模型，本路径用
        // describe 模型（vision 能力）跑完整段——step 2 是纯文本任务，vision 模型也能跑。
        // 想要严格 step 2 用 summary 模型，去 daily 路径重跑当天即可。
        let port = self
            .ensure_engine_running(&ai, Step::Describe, engine_overrides)
            .await?;
        let step1 = ChatClient::new(
            port,
            ai.effective_describe_main().to_string(),
            ai.describe_max_tokens(),
        )?;
        let step2 = build_step2(&ai, port, ai.effective_describe_main())?;
        let max_images = ai.max_images_per_segment as usize;

        // 单段重试是「重新生成段总结」语义，永远走完整 step1+step2 流程
        self.run_one_segment(
            source,
            &step1,
            &step2,
            &ai,
            &date_str,
            segment_idx,
            ai.segments.len() as u32,
            seg.label,
            seg.start_hour,
            seg.end_hour,
            max_images,
            device,
            parallel,
            false,
            false,
        )
        .await
    }

    /// 段内截图按 MobileNet embedding 余弦阈值贪心去重。
    ///
    /// DB 缓存优先（[`screenshot_embeddings`] 表）：命中即取，缺失批量算 + 写表。
    /// embedding 失败兜底为 "跳过去重，原样返回"——dedup 只是提速优化，失败不应让段总结挂掉，
    /// 顶多让 step 1 多跑几张冗余图。
    ///
    /// `threshold` 取 0.70..=0.99，已在 [`crate::ai::config::sanitize`] 钳过。
    async fn dedup_segment_metas(
        &self,
        metas: Vec<ScreenshotMeta>,
        threshold: f32,
    ) -> Result<Vec<ScreenshotMeta>> {
        if metas.len() < 2 {
            return Ok(metas);
        }
        let paths: Vec<String> = metas.iter().map(|m| m.path.clone()).collect();
        let embeddings = match self.load_or_compute_embeddings(&paths).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "dedup 跳过：embedding 失败 ({e}) — 原样返回 {} 张",
                    metas.len()
                );
                return Ok(metas);
            }
        };
        let before = metas.len();
        let kept = dedup::dedup_by_embedding(metas, &embeddings, threshold);
        let dropped = before.saturating_sub(kept.len());
        if before > 0 {
            log::info!(
                "dedup: {before} → {} (drop {} = {:.1}%, threshold={:.2})",
                kept.len(),
                dropped,
                100.0 * dropped as f32 / before as f32,
                threshold,
            );
        }
        Ok(kept)
    }

    /// DB 缓存优先批量取 embedding；缺失的现算并写表。
    /// 返回的向量与 `paths` 一一对齐，长度相等。
    async fn load_or_compute_embeddings(&self, paths: &[String]) -> Result<Vec<Vec<f32>>> {
        let cached = embeddings_repo::get_batch(&self.pool, paths, embedding::MODEL_ID).await?;
        let missing: Vec<PathBuf> = paths
            .iter()
            .filter(|p| !cached.contains_key(*p))
            .map(PathBuf::from)
            .collect();
        log::info!(
            "embedding cache: hit={}, miss={} (model_id={})",
            cached.len(),
            missing.len(),
            embedding::MODEL_ID,
        );

        // 缺失的批量算 + 写表
        let mut newly_computed: std::collections::HashMap<String, Vec<f32>> =
            std::collections::HashMap::new();
        if !missing.is_empty() {
            let new_embs = embedding::compute_batch(&missing).await?;
            let mut rows: Vec<(String, &'static str, Vec<f32>)> = Vec::with_capacity(missing.len());
            for (p, e) in missing.iter().zip(new_embs.iter()) {
                let key = p.to_string_lossy().into_owned();
                newly_computed.insert(key.clone(), e.clone());
                rows.push((key, embedding::MODEL_ID, e.clone()));
            }
            if let Err(e) = embeddings_repo::upsert_batch(&self.pool, rows).await {
                log::warn!("embedding 写表失败（不影响本次去重）：{e}");
            }
        }

        // 按 paths 顺序拼对齐数组
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            let v = cached
                .get(p)
                .or_else(|| newly_computed.get(p))
                .cloned()
                .ok_or_else(|| Error::EmbeddingFailed(format!("missing embedding for {p}")))?;
            out.push(v);
        }
        Ok(out)
    }

    /// 引擎未启动就启动；返回当前监听端口。
    ///
    /// `step` 决定加载哪份模型（describe vs summary，两套各自有 effective fallback
    /// 到 `active_main`）。`engine_overrides` 是该 step 的 batch/-np/ctx 参数；
    /// 调用方在调本函数前应主动 `supervisor.stop()` 触发重启，否则已 Running 时
    /// 直接返回端口、参数不变。
    async fn ensure_engine_running(
        &self,
        ai: &AiConfig,
        step: Step,
        engine_overrides: EngineStartOverrides,
    ) -> Result<u16> {
        let st = self.supervisor.status().await;
        if st.state == EngineState::Running {
            if let Some(p) = st.port {
                return Ok(p);
            }
        }
        let (main_path, mmproj_path) = self.resolve_model_paths_for(ai, step)?;
        self.supervisor
            .start_with_overrides(Some(main_path), mmproj_path, engine_overrides)
            .await
    }

    /// 按 step（describe / summary）解析当前 step 实际要加载的 GGUF 路径。
    /// 文件不存在抛 `ModelFileMissing`，给上层把错误条直接展示给用户。
    fn resolve_model_paths_for(
        &self,
        ai: &AiConfig,
        step: Step,
    ) -> Result<(PathBuf, Option<PathBuf>)> {
        let main_name = match step {
            Step::Describe => ai.effective_describe_main(),
            Step::Summary => ai.effective_summary_main(),
        };
        let mmproj_name = match step {
            Step::Describe => ai.effective_describe_mmproj(),
            Step::Summary => ai.effective_summary_mmproj(),
        };
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
}
