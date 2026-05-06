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
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::ai::config::AiConfig;
use crate::ai::image::{pick_frames, to_data_uri};
use crate::ai::llm::ChatClient;
use crate::ai::models;
use crate::ai::prompt::{build_system_prompt, build_user_prompt, SegmentContext};
use crate::ai::server::EngineSupervisor;
use crate::error::{Error, Result};
use crate::repo::ai_summaries::{
    self, list_segment_screenshots, list_segment_top_apps, SegmentSummaryRow,
};
use crate::repo::reports::DeviceFilter;
use crate::repo::settings as settings_repo;
use crate::storage::DbPool;

/// 前端 listen 这个事件名拿进度。
/// 和 [`crate::commands::ai::PROGRESS_EVENT`] 平级，统一在 `ai://` 命名空间。
pub const SUMMARY_PROGRESS_EVENT: &str = "ai://summary-progress";

/// 进度事件 payload。前端按 `phase` 分发渲染。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryProgress {
    pub date: String,
    /// engine_starting | segment_started | segment_done | all_done | cancelled | error
    pub phase: &'static str,
    pub segment_idx: Option<u32>,
    pub total_segments: u32,
    /// 段开跑时给前端 "12 张图待分析" 的提示
    pub images_total: Option<u32>,
    /// segment_done 时附该段总结，前端立刻渲染该段不等其它
    pub content: Option<String>,
    /// segment_done 时也会带上落库行的 status（ok / skipped_no_screenshots / error）
    /// 让前端知道是不是该段失败了
    pub status: Option<&'static str>,
    /// error 段的可读错误；error phase 也用这个携带顶层错误描述
    pub message: Option<String>,
}

/// 总结 LLM 输出时给段加的图片缩放上限——长边 768 px 是 vision LLM 的常见甜点：
/// 文字仍可读，token 数比原图少一半以上。
const SUMMARY_IMAGE_MAX_DIM: u32 = 768;

/// 一段图最多塞这么多张兜底——即便 settings.ai.max_images_per_segment 写了 30，
/// 实际跑也会被 ctx 4096 撑爆，这里再卡一道。
const HARD_IMAGES_CAP: u32 = 12;

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
    ) -> Result<()> {
        let cfg = settings_repo::load(&self.pool).await?;
        let ai = cfg.ai.clone();

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
        if st.state != "running" {
            self.emit(SummaryProgress {
                date: date_str.clone(),
                phase: "engine_starting",
                segment_idx: None,
                total_segments,
                images_total: None,
                content: None,
                status: None,
                message: Some("加载模型中（首次约 30-90 秒）…".to_string()),
            });
        }
        let port = self.ensure_engine_running(&ai).await?;
        let chat = ChatClient::new(port, ai.active_main.clone())?;

        // 单段图片上限：settings 配的值再卡 HARD_IMAGES_CAP
        let max_images = (ai.max_images_per_segment as u32).min(HARD_IMAGES_CAP) as usize;

        for (idx, seg) in ai.segments.iter().enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                self.emit(SummaryProgress {
                    date: date_str.clone(),
                    phase: "cancelled",
                    segment_idx: Some(idx as u32),
                    total_segments,
                    images_total: None,
                    content: None,
                    status: None,
                    message: None,
                });
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

        self.emit(SummaryProgress {
            date: date_str,
            phase: "all_done",
            segment_idx: None,
            total_segments,
            images_total: None,
            content: None,
            status: None,
            message: None,
        });
        Ok(())
    }

    fn emit(&self, payload: SummaryProgress) {
        if let Err(e) = self.app.emit(SUMMARY_PROGRESS_EVENT, &payload) {
            log::warn!("emit {SUMMARY_PROGRESS_EVENT} 失败: {e}");
        }
    }

    /// 跑单段，把结果落库（无论 ok / skipped / error 都写一行）+ emit 进度事件。
    /// 上层错误（DB 操作）会向上抛；段内 LLM 错误会被捕获写成 status='error' 而不抛。
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

        // 拉段内截图路径 + top apps
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
            self.emit(SummaryProgress {
                date: date_str.to_string(),
                phase: "segment_started",
                segment_idx: Some(idx),
                total_segments,
                images_total: Some(0),
                content: None,
                status: None,
                message: None,
            });
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
            self.emit(SummaryProgress {
                date: date_str.to_string(),
                phase: "segment_done",
                segment_idx: Some(idx),
                total_segments,
                images_total: Some(0),
                content: Some(String::new()),
                status: Some("skipped_no_screenshots"),
                message: None,
            });
            return Ok(());
        }

        // 等距抽帧
        let picked = pick_frames(paths, max_images);
        self.emit(SummaryProgress {
            date: date_str.to_string(),
            phase: "segment_started",
            segment_idx: Some(idx),
            total_segments,
            images_total: Some(picked.len() as u32),
            content: None,
            status: None,
            message: None,
        });

        // 每张转 data URI；个别坏文件 log 一下跳过，不致命
        let mut data_uris: Vec<String> = Vec::with_capacity(picked.len());
        for p in &picked {
            match to_data_uri(Path::new(p), SUMMARY_IMAGE_MAX_DIM).await {
                Ok(uri) => data_uris.push(uri),
                Err(e) => log::warn!("跳过坏截图 {p}: {e}"),
            }
        }

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

        let ctx = SegmentContext {
            label: &label,
            start_hour,
            end_hour,
            top_apps: &top_apps,
            image_count: data_uris.len(),
        };
        let system = build_system_prompt(ai);
        let user_text = build_user_prompt(ai, &ctx);

        let (row, status_str): (SegmentSummaryRow, &'static str) =
            match chat.chat_with_images(&system, &user_text, &data_uris).await {
                Ok(content) => (
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
        self.emit(SummaryProgress {
            date: date_str.to_string(),
            phase: "segment_done",
            segment_idx: Some(idx),
            total_segments,
            images_total: Some(data_uris.len() as u32),
            content: Some(row.content.clone()),
            status: Some(status_str),
            message: row.error.clone(),
        });
        Ok(())
    }

    /// "重试某段"专用：只跑指定一段，复用现有引擎。
    #[allow(clippy::too_many_arguments)]
    pub async fn run_one_segment_only(
        &self,
        local_date: NaiveDate,
        segment_idx: u32,
        device: DeviceFilter,
    ) -> Result<()> {
        let cfg = settings_repo::load(&self.pool).await?;
        let ai = cfg.ai.clone();

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
        if st.state != "running" {
            self.emit(SummaryProgress {
                date: date_str.clone(),
                phase: "engine_starting",
                segment_idx: None,
                total_segments: ai.segments.len() as u32,
                images_total: None,
                content: None,
                status: None,
                message: Some("加载模型中（首次约 30-90 秒）…".to_string()),
            });
        }
        let port = self.ensure_engine_running(&ai).await?;
        let chat = ChatClient::new(port, ai.active_main.clone())?;
        let max_images =
            (ai.max_images_per_segment as u32).min(HARD_IMAGES_CAP) as usize;

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
        if st.state == "running" {
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
