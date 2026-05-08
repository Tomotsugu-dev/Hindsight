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
use crate::repo::reports::DeviceFilter;
use crate::repo::settings as settings_repo;
use crate::storage::DbPool;

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

        let engine_overrides = EngineStartOverrides {
            batch_size: ai.batch_size,
            parallel_slots: ai.parallel_slots,
            ctx_size: ai.ctx_size,
        };
        let parallel = ai.parallel_slots.unwrap_or(1).max(1) as usize;

        if ai.active_main.trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」选一个 vision 模型再生成总结",
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

        // 启动引擎（如未启动），拿到端口；启动期间给前端一个进度提示
        let st = self.supervisor.status().await;
        if st.state != EngineState::Running {
            let mut p = SummaryProgress::base(
                source.to_string(),
                date_str.clone(),
                "engine_starting",
                total_segments,
            );
            p.message = Some("加载模型中（首次约 30-90 秒）…".to_string());
            self.emit(p);
        }
        let port = self.ensure_engine_running(&ai, engine_overrides).await?;
        let step1 = ChatClient::new(port, ai.active_main.clone())?;
        let step2 = build_step2(&ai, &step1)?;

        // 单段图片上限直接走 settings；用户调大撑爆 ctx 时 LLM 报 400，按段标 error
        let max_images = ai.max_images_per_segment as usize;

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
            // 防御：sanitize 已确保 start < end，这里只是兜底
            if seg.end_hour <= seg.start_hour {
                continue;
            }

            self.run_one_segment(
                source,
                &step1,
                &step2,
                &ai,
                &date_str,
                idx as u32,
                total_segments,
                seg.label.clone(),
                seg.start_hour,
                seg.end_hour,
                max_images,
                device.clone(),
                parallel,
                step1_only,
                step2_only,
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

        // step 1 落库的 model 永远是本地 vision 文件名；step 2 落库的 model
        // 由 step2.model_label() 给出（本地 = active_main，外部 = 用户填的 ID）
        let model = ai.active_main.clone();
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

        // step1_only：跳过 step 2 的段总结，直接 emit 一条 segment_done 让前端结束 loading。
        // status 用 "ok"——逐图描述都已落库，对前端来说本段已完成。content 留空（没段总结正文）。
        // 不写 ai_summaries 行：避免后续 daily 跑用户期待"重新跑段总结"时被旧空 row 挡住。
        if step1_only {
            let mut p_done = SummaryProgress::base(
                source.to_string(),
                date_str.to_string(),
                "segment_done",
                total_segments,
            );
            p_done.segment_idx = Some(idx);
            p_done.images_total = Some(picked.len() as u32);
            p_done.content = Some(String::new());
            p_done.status = Some("ok");
            self.emit(p_done);
            return Ok(());
        }

        // ────── step 2：段总结（纯文本调用，不再传图） ──────
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
    /// 没存数据 → 按 skipped 兜底（跟 step1 路径下"没截图"语义对齐）。
    /// 用户调试段总结 prompt 时反复调，避免每次都重跑昂贵的 vision 描述。
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
        let model = ai.active_main.clone();
        let step2_model = step2.model_label().to_string();

        let stored =
            ai_summaries::get_segment_image_descriptions(&self.pool, source, date_str, idx).await?;

        let mut p_started = SummaryProgress::base(
            source.to_string(),
            date_str.to_string(),
            "segment_started",
            total_segments,
        );
        p_started.segment_idx = Some(idx);
        p_started.images_total = Some(stored.len() as u32);
        self.emit(p_started);

        if stored.is_empty() {
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

        if ai.active_main.trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」选一个 vision 模型再生成总结",
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
        // 启引擎（如未启动）
        let port = self.ensure_engine_running(&ai, engine_overrides).await?;
        let chat = ChatClient::new(port, ai.active_main.clone())?;

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
                model: ai.active_main.clone(),
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

        if ai.active_main.trim().is_empty() {
            return Err(Error::InvalidInput(
                "请先在「模型」选一个 vision 模型再生成总结",
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
        let port = self.ensure_engine_running(&ai, engine_overrides).await?;
        let step1 = ChatClient::new(port, ai.active_main.clone())?;
        let step2 = build_step2(&ai, &step1)?;
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

    /// 引擎未启动就启动。返回当前监听端口。
    ///
    /// `engine_overrides` 在调用方决定要不要重启 / 用什么参数启动；这里只负责：
    /// 引擎已 Running 就返回端口，否则按 overrides 启动。如果调用方想改 batch
    /// / parallel_slots，应在调本函数前主动 `supervisor.stop()` 一下，让本函
    /// 数走启动分支并带着 overrides 上去。
    async fn ensure_engine_running(
        &self,
        ai: &AiConfig,
        engine_overrides: EngineStartOverrides,
    ) -> Result<u16> {
        let st = self.supervisor.status().await;
        if st.state == EngineState::Running {
            if let Some(p) = st.port {
                return Ok(p);
            }
        }

        let models_dir = models::root_dir(ai);
        let main_path: PathBuf = models_dir.join(&ai.active_main);
        if !main_path.exists() {
            return Err(Error::ModelFileMissing(format!(
                "{}（可能被删除或路径变了）",
                ai.active_main
            )));
        }
        let mmproj_path = if ai.active_mmproj.trim().is_empty() {
            None
        } else {
            let p = models_dir.join(&ai.active_mmproj);
            if !p.exists() {
                return Err(Error::ModelFileMissing(format!(
                    "vision 投影 {}",
                    ai.active_mmproj
                )));
            }
            Some(p)
        };
        self.supervisor
            .start_with_overrides(Some(main_path), mmproj_path, engine_overrides)
            .await
    }
}
