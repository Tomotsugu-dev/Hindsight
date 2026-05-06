//! AI 总结的编排核心（Phase 1B-γ）。
//!
//! [`DaySummaryRunner::run`] 是单点入口：拿到一天 + 设备过滤 + 是否强刷，
//! 内部按 settings.ai.segments 切段，串行跑每一段：
//!
//!   1. 拉段内截图 + top apps
//!   2. 截图等距下采样到 max_images_per_segment 张
//!   3. 每张转成 data URI
//!   4. 拼 prompt + 调 ChatClient
//!   5. 把段总结落 DB（status: ok / skipped_no_screenshots / error）
//!
//! γ.5 会加：cancel token + 进度事件 emit。当前 γ.4 是最小可工作版本，
//! 命令行直跑能在 DB 里看到结果。
//!
//! 串行而非并发：本地 llama-server 是单实例，并发请求只会让 llama.cpp 内部排队。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::ai::config::AiConfig;
use crate::ai::image::{pick_frames, to_data_uri};
use crate::ai::llm::ChatClient;
use crate::ai::models;
use crate::ai::prompt::{
    build_image_describe_system_prompt, build_image_describe_user_prompt,
    build_system_prompt, build_user_prompt, SegmentContext,
};
use crate::ai::server::{EngineState, EngineSupervisor};
use crate::error::{Error, Result};
use crate::repo::ai_summaries::{
    self, list_segment_screenshots, list_segment_top_apps, ImageDescriptionRow,
    SegmentSummaryRow,
};
use crate::repo::reports::DeviceFilter;
use crate::repo::settings as settings_repo;
use crate::storage::DbPool;

/// 调试用：单次 generate 调用对 settings.ai 的局部覆盖（不写 settings 全局）。
///
/// 任意字段 `None` = 走 settings.ai 的值；`Some(_)` = 本次跑生效，不留痕。
/// 数值字段会经过跟 sanitize 一样的 clamp（max_images 1..=200、hash_threshold ≤ 32、
/// hash_window_minutes ≤ 60），保证 override 不会越界。
///
/// `system_prompt` / `image_describe_prompt` 是文本覆盖，会写到 ai.prompt_overrides /
/// ai.image_describe_overrides 当前语言对应的字段——空字符串等价"清覆盖走默认"。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOverrides {
    pub excluded_categories: Option<Vec<String>>,
    pub max_images_per_segment: Option<u32>,
    pub hash_threshold: Option<u32>,
    pub hash_window_minutes: Option<u32>,
    /// step 2 段总结的 system prompt 覆盖文本（按当前语言写入 prompt_overrides）
    pub system_prompt: Option<String>,
    /// step 1 单图描述的 system prompt 覆盖文本（按当前语言写入 image_describe_overrides）
    pub image_describe_prompt: Option<String>,
}

impl AiOverrides {
    /// 把 override 应用到 settings.ai 上；clamp 到合法区间后返回。
    fn apply_to(self, mut ai: AiConfig) -> AiConfig {
        if let Some(v) = self.excluded_categories {
            ai.excluded_categories = v;
        }
        if let Some(v) = self.max_images_per_segment {
            ai.max_images_per_segment = v.clamp(1, 200);
        }
        if let Some(v) = self.hash_threshold {
            ai.hash_threshold = v.min(32);
        }
        if let Some(v) = self.hash_window_minutes {
            ai.hash_window_minutes = v.min(60);
        }
        // 文本 prompt 覆盖按当前 prompt_language 写到对应字段；空串等同 "走默认"
        let lang = ai.prompt_language.clone();
        if let Some(v) = self.system_prompt {
            match lang.as_str() {
                "en" => ai.prompt_overrides.system_en = v,
                "ja" => ai.prompt_overrides.system_ja = v,
                _ => ai.prompt_overrides.system_zh = v,
            }
        }
        if let Some(v) = self.image_describe_prompt {
            match lang.as_str() {
                "en" => ai.image_describe_overrides.system_en = v,
                "ja" => ai.image_describe_overrides.system_ja = v,
                _ => ai.image_describe_overrides.system_zh = v,
            }
        }
        ai
    }
}

/// 前端 listen 这个事件名拿进度。
/// 和 [`crate::commands::ai::PROGRESS_EVENT`] 平级，统一在 `ai://` 命名空间。
pub const SUMMARY_PROGRESS_EVENT: &str = "ai://summary-progress";

/// 进度事件 payload。前端按 `phase` 分发渲染。
///
/// phase 取值：
/// - `engine_starting`：引擎冷启动中（首次加载模型 30-90s）
/// - `segment_started`：段进入 step 1（逐图描述）；imagesTotal 给图数
/// - `image_described`：单张图描述完成；image_index / image_path / image_description 一起带过来，
///   前端调试 tab 实时往面板里塞条目，不必等整段完成
/// - `segment_done`：段进入完成态（含 ok / skipped / error）；content 是段总结
/// - `all_done` / `cancelled` / `error`：整轮收尾
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryProgress {
    pub date: String,
    pub phase: &'static str,
    pub segment_idx: Option<u32>,
    pub total_segments: u32,
    /// 段开跑时给前端 "12 张图待分析" 的提示
    pub images_total: Option<u32>,
    /// image_described 时该图在段内的下标（0-based）
    pub image_index: Option<u32>,
    /// image_described 时附该图绝对路径（前端可以用来显示缩略图）
    pub image_path: Option<String>,
    /// image_described 时附该图的描述文本
    pub image_description: Option<String>,
    /// image_described 时附该图调用 LLM 的耗时（毫秒）
    pub latency_ms: Option<u64>,
    /// image_described 时附 prompt token 数（llama-server 不返时为 None）
    pub prompt_tokens: Option<u32>,
    /// image_described 时附 completion token 数
    pub completion_tokens: Option<u32>,
    /// segment_done 时附该段总结，前端立刻渲染该段不等其它
    pub content: Option<String>,
    /// segment_done 时也会带上落库行的 status（ok / skipped_no_screenshots / error）
    /// 让前端知道是不是该段失败了
    pub status: Option<&'static str>,
    /// error 段的可读错误；error phase 也用这个携带顶层错误描述
    pub message: Option<String>,
}

impl SummaryProgress {
    /// 各 phase 字段大多都是 None，统一兜底构造器减少重复代码。
    pub(crate) fn base(date: String, phase: &'static str, total_segments: u32) -> Self {
        Self {
            date,
            phase,
            segment_idx: None,
            total_segments,
            images_total: None,
            image_index: None,
            image_path: None,
            image_description: None,
            latency_ms: None,
            prompt_tokens: None,
            completion_tokens: None,
            content: None,
            status: None,
            message: None,
        }
    }
}

/// 总结 LLM 输出时给段加的图片缩放上限——长边 768 px 是 vision LLM 的常见甜点：
/// 文字仍可读，token 数比原图少一半以上。
const SUMMARY_IMAGE_MAX_DIM: u32 = 768;

// HARD_IMAGES_CAP 已取消——一段图数量完全由 settings.ai.max_images_per_segment 决定
// （sanitize 钳到 1..=200）。配多了 ctx 装不下时 LLM 会返 400，按段标 error，
// 用户在调试 tab 能直接看到错误并调小值。

pub struct DaySummaryRunner {
    pool: DbPool,
    supervisor: Arc<EngineSupervisor>,
    app: AppHandle,
    /// 取消信号：每段开跑前检查；true 时整轮停止（落库写到了哪段就到哪段）。
    /// 不能中断已经在路上的 LLM 请求——一段 30-180s 的 chat 必须跑完才能 yield。
    cancel: Arc<AtomicBool>,
}

impl DaySummaryRunner {
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
    pub async fn run(
        &self,
        local_date: NaiveDate,
        device: DeviceFilter,
        force_refresh: bool,
        overrides: Option<AiOverrides>,
    ) -> Result<()> {
        let cfg = settings_repo::load(&self.pool).await?;
        // overrides 只对本次调用生效，不写回 settings；settings 自己永远是用户在
        // AI 设置里配的"全局值"，调试 tab 改的本地参数从这里进来
        let ai = match overrides {
            Some(o) => o.apply_to(cfg.ai.clone()),
            None => cfg.ai.clone(),
        };

        if ai.active_main.trim().is_empty() {
            return Err(Error::Other(
                "请先在「模型」选一个 vision 模型再生成总结".to_string(),
            ));
        }

        let date_str = local_date.format("%Y-%m-%d").to_string();
        if force_refresh {
            ai_summaries::clear_day(&self.pool, &date_str).await?;
        }

        let total_segments = ai.segments.len() as u32;

        // 启动引擎（如未启动），拿到端口；启动期间给前端一个进度提示
        let st = self.supervisor.status().await;
        if st.state != EngineState::Running {
            let mut p = SummaryProgress::base(date_str.clone(), "engine_starting", total_segments);
            p.message = Some("加载模型中（首次约 30-90 秒）…".to_string());
            self.emit(p);
        }
        let port = self.ensure_engine_running(&ai).await?;
        let chat = ChatClient::new(port, ai.active_main.clone())?;

        // 单段图片上限直接走 settings；用户调大撑爆 ctx 时 LLM 报 400，按段标 error
        let max_images = ai.max_images_per_segment as usize;

        for (idx, seg) in ai.segments.iter().enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                let mut p =
                    SummaryProgress::base(date_str.clone(), "cancelled", total_segments);
                p.segment_idx = Some(idx as u32);
                self.emit(p);
                return Ok(());
            }
            // 防御：sanitize 已确保 start < end，这里只是兜底
            if seg.end_hour <= seg.start_hour {
                continue;
            }

            self.run_one_segment(
                &chat,
                &ai,
                &date_str,
                idx as u32,
                total_segments,
                seg.label.clone(),
                seg.start_hour,
                seg.end_hour,
                max_images,
                device.clone(),
            )
            .await?;
        }

        let p = SummaryProgress::base(date_str, "all_done", total_segments);
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
        chat: &ChatClient,
        ai: &AiConfig,
        date_str: &str,
        idx: u32,
        total_segments: u32,
        label: String,
        start_hour: u8,
        end_hour: u8,
        max_images: usize,
        device: DeviceFilter,
    ) -> Result<()> {
        let model = ai.active_main.clone();

        // ────── 取数据 ──────
        let paths = list_segment_screenshots(
            &self.pool,
            date_str,
            start_hour,
            end_hour,
            &ai.excluded_categories,
            device.clone(),
        )
        .await?;

        // 没截图的段直接 skipped 兜底
        if paths.is_empty() {
            let mut p_started =
                SummaryProgress::base(date_str.to_string(), "segment_started", total_segments);
            p_started.segment_idx = Some(idx);
            p_started.images_total = Some(0);
            self.emit(p_started);

            ai_summaries::upsert_segment(
                &self.pool,
                &SegmentSummaryRow {
                    local_date: date_str.to_string(),
                    segment_idx: idx,
                    label: label.clone(),
                    start_hour,
                    end_hour,
                    content: String::new(),
                    model,
                    status: "skipped_no_screenshots".to_string(),
                    error: None,
                    generated_at: Utc::now().to_rfc3339(),
                },
            )
            .await?;

            let mut p_done =
                SummaryProgress::base(date_str.to_string(), "segment_done", total_segments);
            p_done.segment_idx = Some(idx);
            p_done.images_total = Some(0);
            p_done.content = Some(String::new());
            p_done.status = Some("skipped_no_screenshots");
            self.emit(p_done);
            return Ok(());
        }

        // 等距抽帧
        let picked = pick_frames(paths, max_images);

        // 段重跑前先清掉旧的逐图描述，避免新旧 image_index 错位
        ai_summaries::clear_segment_descriptions(&self.pool, date_str, idx).await?;

        let mut p_started =
            SummaryProgress::base(date_str.to_string(), "segment_started", total_segments);
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
        let describe_system = build_image_describe_system_prompt(ai);
        let describe_user =
            build_image_describe_user_prompt(ai, &label, start_hour, end_hour);
        let mut descriptions: Vec<String> = Vec::with_capacity(picked.len());

        for (i, img_path) in picked.iter().enumerate() {
            // 提早响应取消（不打断已在路上的 chat，下一张不开始）
            if self.cancel.load(Ordering::Relaxed) {
                let mut p_cancel =
                    SummaryProgress::base(date_str.to_string(), "cancelled", total_segments);
                p_cancel.segment_idx = Some(idx);
                self.emit(p_cancel);
                return Ok(());
            }

            // 单张转 data URI；坏文件跳过
            let data_uri = match to_data_uri(Path::new(img_path), SUMMARY_IMAGE_MAX_DIM).await {
                Ok(u) => u,
                Err(e) => {
                    log::warn!("跳过坏截图 {img_path}: {e}");
                    continue;
                }
            };

            // 单图调用 LLM；失败 log 跳过，不阻塞整段
            let single = std::slice::from_ref(&data_uri);
            let (desc, usage) = match chat
                .chat_with_images(&describe_system, &describe_user, single)
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("图描述失败 {img_path}: {e}");
                    continue;
                }
            };

            // 落库
            ai_summaries::upsert_image_description(
                &self.pool,
                &ImageDescriptionRow {
                    local_date: date_str.to_string(),
                    segment_idx: idx,
                    image_index: i as u32,
                    screenshot_path: img_path.clone(),
                    description: desc.clone(),
                    model: model.clone(),
                    generated_at: Utc::now().to_rfc3339(),
                    latency_ms: Some(usage.latency_ms),
                    prompt_tokens: usage.prompt_tokens,
                    completion_tokens: usage.completion_tokens,
                },
            )
            .await?;

            // emit 给前端调试 tab 实时渲染
            let mut p_img = SummaryProgress::base(
                date_str.to_string(),
                "image_described",
                total_segments,
            );
            p_img.segment_idx = Some(idx);
            p_img.image_index = Some(i as u32);
            p_img.image_path = Some(img_path.clone());
            p_img.image_description = Some(desc.clone());
            p_img.latency_ms = Some(usage.latency_ms);
            p_img.prompt_tokens = usage.prompt_tokens;
            p_img.completion_tokens = usage.completion_tokens;
            self.emit(p_img);

            descriptions.push(desc);
        }

        // ────── step 2：段总结（纯文本调用，不再传图） ──────
        let ctx = SegmentContext {
            label: &label,
            start_hour,
            end_hour,
            top_apps: &top_apps,
            image_descriptions: &descriptions,
        };
        let system = build_system_prompt(ai);
        let user_text = build_user_prompt(ai, &ctx);

        let (row, status_str): (SegmentSummaryRow, &'static str) =
            match chat.chat_with_images(&system, &user_text, &[]).await {
                // step 2 是纯文本调用，本表只关心 content；usage 暂不落 ai_summaries
                // （需要时未来加列）
                Ok((content, _usage)) => (
                    SegmentSummaryRow {
                        local_date: date_str.to_string(),
                        segment_idx: idx,
                        label: label.clone(),
                        start_hour,
                        end_hour,
                        content,
                        model,
                        status: "ok".to_string(),
                        error: None,
                        generated_at: Utc::now().to_rfc3339(),
                    },
                    "ok",
                ),
                Err(e) => (
                    SegmentSummaryRow {
                        local_date: date_str.to_string(),
                        segment_idx: idx,
                        label: label.clone(),
                        start_hour,
                        end_hour,
                        content: String::new(),
                        model,
                        status: "error".to_string(),
                        error: Some(e.to_string()),
                        generated_at: Utc::now().to_rfc3339(),
                    },
                    "error",
                ),
            };

        ai_summaries::upsert_segment(&self.pool, &row).await?;

        let mut p_done =
            SummaryProgress::base(date_str.to_string(), "segment_done", total_segments);
        p_done.segment_idx = Some(idx);
        p_done.images_total = Some(picked.len() as u32);
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
        local_date: NaiveDate,
        segment_idx: u32,
        image_index: u32,
        overrides: Option<AiOverrides>,
    ) -> Result<()> {
        let cfg = settings_repo::load(&self.pool).await?;
        let ai = match overrides {
            Some(o) => o.apply_to(cfg.ai.clone()),
            None => cfg.ai.clone(),
        };

        if ai.active_main.trim().is_empty() {
            return Err(Error::Other(
                "请先在「模型」选一个 vision 模型再生成总结".to_string(),
            ));
        }

        let date_str = local_date.format("%Y-%m-%d").to_string();
        let total_segments = ai.segments.len() as u32;

        // 拉该段该 image_index 的现有行（拿 screenshot_path）
        let existing = ai_summaries::get_segment_image_descriptions(
            &self.pool,
            &date_str,
            segment_idx,
        )
        .await?;
        let existing_row = existing
            .into_iter()
            .find(|r| r.image_index == image_index)
            .ok_or_else(|| {
                Error::Other(format!(
                    "段 {} 第 {} 张图没有现有描述，先跑一次完整生成",
                    segment_idx, image_index
                ))
            })?;

        // 启引擎（如未启动）
        let port = self.ensure_engine_running(&ai).await?;
        let chat = ChatClient::new(port, ai.active_main.clone())?;

        // 取段标签 / 时段
        let seg = ai.segments.get(segment_idx as usize).ok_or_else(|| {
            Error::Other(format!("段下标越界：{}", segment_idx))
        })?;

        let describe_system = build_image_describe_system_prompt(&ai);
        let describe_user = build_image_describe_user_prompt(
            &ai,
            &seg.label,
            seg.start_hour,
            seg.end_hour,
        );

        let data_uri = to_data_uri(
            Path::new(&existing_row.screenshot_path),
            SUMMARY_IMAGE_MAX_DIM,
        )
        .await?;

        let (desc, usage) = chat
            .chat_with_images(&describe_system, &describe_user, std::slice::from_ref(&data_uri))
            .await?;

        // 覆盖落库
        ai_summaries::upsert_image_description(
            &self.pool,
            &ImageDescriptionRow {
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
        let mut p = SummaryProgress::base(date_str, "image_described", total_segments);
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
    #[allow(clippy::too_many_arguments)]
    pub async fn run_one_segment_only(
        &self,
        local_date: NaiveDate,
        segment_idx: u32,
        device: DeviceFilter,
        overrides: Option<AiOverrides>,
    ) -> Result<()> {
        let cfg = settings_repo::load(&self.pool).await?;
        let ai = match overrides {
            Some(o) => o.apply_to(cfg.ai.clone()),
            None => cfg.ai.clone(),
        };

        if ai.active_main.trim().is_empty() {
            return Err(Error::Other(
                "请先在「模型」选一个 vision 模型再生成总结".to_string(),
            ));
        }

        let seg = ai
            .segments
            .get(segment_idx as usize)
            .cloned()
            .ok_or_else(|| Error::Other(format!("段下标越界：{}", segment_idx)))?;
        if seg.end_hour <= seg.start_hour {
            return Err(Error::Other("段时间范围非法".into()));
        }

        let date_str = local_date.format("%Y-%m-%d").to_string();

        let st = self.supervisor.status().await;
        if st.state != EngineState::Running {
            let mut p = SummaryProgress::base(
                date_str.clone(),
                "engine_starting",
                ai.segments.len() as u32,
            );
            p.message = Some("加载模型中（首次约 30-90 秒）…".to_string());
            self.emit(p);
        }
        let port = self.ensure_engine_running(&ai).await?;
        let chat = ChatClient::new(port, ai.active_main.clone())?;
        let max_images = ai.max_images_per_segment as usize;

        self.run_one_segment(
            &chat,
            &ai,
            &date_str,
            segment_idx,
            ai.segments.len() as u32,
            seg.label,
            seg.start_hour,
            seg.end_hour,
            max_images,
            device,
        )
        .await
    }

    /// 引擎未启动就启动。返回当前监听端口。
    /// 复制了 `commands::ai::start_engine` 里的路径组装逻辑，因为那里是
    /// `#[tauri::command]` 不能直接调用。
    async fn ensure_engine_running(&self, ai: &AiConfig) -> Result<u16> {
        let st = self.supervisor.status().await;
        if st.state == EngineState::Running {
            if let Some(p) = st.port {
                return Ok(p);
            }
        }

        let models_dir = models::root_dir(ai);
        let main_path: PathBuf = models_dir.join(&ai.active_main);
        if !main_path.exists() {
            return Err(Error::Other(format!(
                "选中的主权重文件不存在：{}（可能被删除或路径变了）",
                ai.active_main
            )));
        }
        let mmproj_path = if ai.active_mmproj.trim().is_empty() {
            None
        } else {
            let p = models_dir.join(&ai.active_mmproj);
            if !p.exists() {
                return Err(Error::Other(format!(
                    "选中的 vision 投影文件不存在：{}",
                    ai.active_mmproj
                )));
            }
            Some(p)
        };
        self.supervisor.start(Some(main_path), mmproj_path).await
    }
}
