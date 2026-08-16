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

/// 进度事件出口的最小抽象。生产代码里就是 [`AppHandle`]（emit 给前端）；
/// 抽成 trait 是为了让编排逻辑能脱离 Tauri 运行时做单元测试——`tauri` 依赖
/// 没开 `test` feature，测试进程里根本构造不出 AppHandle，这是 runner
/// 对 Tauri 的唯一硬依赖，砍断它其余全部编排都可测。
pub trait ProgressSink {
    fn emit_progress(&self, payload: SummaryProgress);
}

impl ProgressSink for AppHandle {
    fn emit_progress(&self, payload: SummaryProgress) {
        if let Err(e) = self.emit(SUMMARY_PROGRESS_EVENT, &payload) {
            log::warn!("emit {SUMMARY_PROGRESS_EVENT} 失败: {e}");
        }
    }
}

/// 单点入口：跑一天的 AI 总结。lib.rs 通过 `app.manage` 不直接管它，
/// 而是命令体里临时构造（生命周期跟随单次调用）。
///
/// 泛型参数默认 [`AppHandle`]：所有生产调用点 `DaySummaryRunner::new(.., app, ..)`
/// 不需要任何改动；测试里换成事件收集器。
pub struct DaySummaryRunner<S: ProgressSink = AppHandle> {
    pool: DbPool,
    supervisor: Arc<EngineSupervisor>,
    app: S,
    /// 取消信号：段边界检查 + 在途 LLM 请求和引擎加载也会被
    /// [`crate::ai::summary_operations::cancellable`] 每 250ms 轮询中断。
    cancel: Arc<AtomicBool>,
}

impl<S: ProgressSink> DaySummaryRunner<S> {
    /// 由命令体临时构造（不进 Tauri State）：每次跑一次总结时新建一份。
    /// `cancel` 来自全局 `SummaryCancel` 单例，前端调 cancel_day_summary 时被设 true。
    pub fn new(
        pool: DbPool,
        supervisor: Arc<EngineSupervisor>,
        app: S,
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
        // 这里**不**前置 clear_day。删在重建之前意味着:模型文件缺失、密钥过期、
        // 端点不可用时,当天已有的好报告先没了、重建又起不来,归零且无从恢复。
        // upsert_segment 的 ON CONFLICT 本就原地覆盖,重建不需要先删;删除的唯一
        // 用途是清理"段配置缩小/改非法"后的孤儿行,那件事挪到整轮跑完后按实际
        // 有效段集合收敛(见循环后的 prune_segments_except)。
        // force_refresh 在下面退化为"不复用已 ok 的段",语义不变。

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

        // 本轮实际有效的段下标——循环结束后用它收敛掉孤儿行
        let mut kept: Vec<u32> = Vec::new();
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
                // 非法段:不进 kept,循环后会被 prune 掉(旧实现靠前置 clear_day 顺带清)
                continue;
            }
            kept.push(idx as u32);
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

        // 整轮跑完(未被取消/未提前返错)才收敛:删除永远发生在新内容落库之后。
        // 失败只告警——清理不掉孤儿段不影响本次生成的结果。
        match ai_summaries::prune_segments_except(&self.pool, source, &date_str, &kept).await {
            Ok(n) if n > 0 => log::info!("{source} {date_str}: 清理 {n} 条失效段"),
            Ok(_) => {}
            Err(e) => log::warn!("{source} {date_str}: 清理失效段失败(不影响本次结果): {e}"),
        }

        let p = SummaryProgress::base(source.to_string(), date_str, "all_done", total_segments);
        self.emit(p);
        Ok(())
    }

    fn emit(&self, payload: SummaryProgress) {
        self.app.emit_progress(payload);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use crate::ai::config::{AiSegment, SUMMARY_CLOUD_SENTINEL};
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use crate::storage::SqliteResultExt;

    // ─────────────────── 测试基建：进度收集 / 假云端 / 数据种子 ───────────────────
    //
    // 编排测试的思路：settings 里把段总结路由到「云端」，endpoint 指向本地
    // 127.0.0.1 的脚本化假 OpenAI 服务——这样 runner 的引擎管理分支（本地
    // llama-server 启停）天然短路，其余编排（切段 / 材料合成 / 落库 / 进度事件 /
    // 取消 / 失败续跑）全部走真代码真 DB。

    /// [`ProgressSink`] 的测试实现：把每个进度事件按序收进 Vec。
    /// `cancel_on_segment_done` 模拟「用户看到某段完成后按停止」——在 emit
    /// segment_done 的同一瞬间置位取消标志，检验段边界的取消检查。
    #[derive(Clone, Default)]
    struct RecordingSink {
        events: Arc<Mutex<Vec<SummaryProgress>>>,
        cancel_on_segment_done: Option<Arc<AtomicBool>>,
    }

    impl ProgressSink for RecordingSink {
        fn emit_progress(&self, payload: SummaryProgress) {
            if payload.phase == "segment_done" {
                if let Some(c) = &self.cancel_on_segment_done {
                    c.store(true, Ordering::Relaxed);
                }
            }
            self.events.lock().unwrap().push(payload);
        }
    }

    /// 收集到的事件压成 (phase, segment_idx) 序列，给断言比对用。
    fn phase_seq(events: &[SummaryProgress]) -> Vec<(&'static str, Option<u32>)> {
        events.iter().map(|p| (p.phase, p.segment_idx)).collect()
    }

    /// 脚本化假云端的单发行为。
    enum Canned {
        /// 回一发 OpenAI 兼容 200，content 为给定文本
        Ok(&'static str),
        /// 回 500——模拟云端内部错误（连接是通的，业务失败）
        Http500,
    }

    /// 起一个脚本化的本地假 OpenAI 服务：第 N 个连接消费脚本第 N 项；
    /// 脚本耗尽后一律回 500——这样「不应该发出的多余请求」会以 error 行
    /// 的形式在断言里暴露出来，而不是静默成功。
    /// 返回 (端口, 已捕获的请求 body 列表)——body 用来断言材料过滤真的生效。
    async fn spawn_scripted_openai_server(script: Vec<Canned>) -> (u16, Arc<Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let bodies_in = bodies.clone();
        tokio::spawn(async move {
            let mut script = VecDeque::from(script);
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                // 读完整个请求（headers + Content-Length body）再回包，
                // 避免半途关闭触发 RST 让客户端报奇怪的连接错误
                let mut buf: Vec<u8> = Vec::new();
                let mut tmp = [0u8; 4096];
                let mut body_start = 0usize;
                while let Ok(n) = sock.read(&mut tmp).await {
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let cl = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if buf.len() >= pos + 4 + cl {
                            body_start = pos + 4;
                            break;
                        }
                    }
                }
                bodies_in
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[body_start..]).into_owned());
                let resp = match script.pop_front().unwrap_or(Canned::Http500) {
                    Canned::Ok(content) => {
                        let body = serde_json::json!({
                            "choices": [{
                                "message": { "role": "assistant", "content": content },
                                "finish_reason": "stop"
                            }],
                            "usage": { "prompt_tokens": 42, "completion_tokens": 17 }
                        })
                        .to_string();
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                    }
                    Canned::Http500 => {
                        let body = r#"{"error":"internal"}"#;
                        format!(
                            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                    }
                };
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (port, bodies)
    }

    /// 拿一个刚刚释放的本地端口——连接必然被拒。给「不应发出任何请求」的
    /// 场景当 endpoint：若 runner 意外发了请求，会落 error 行被断言抓住。
    fn free_local_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    fn seg(label: &str, start_hour: u8, end_hour: u8) -> AiSegment {
        AiSegment {
            label: label.to_string(),
            start_hour,
            end_hour,
            color: String::new(),
        }
    }

    /// 把「云端段总结 + 指定段划分 + 排除分类」写进 settings_store。
    /// runner 的 run_inner 每次都从 DB 现读 settings——这是唯一的注入口。
    async fn seed_cloud_settings(
        pool: &DbPool,
        port: u16,
        segments: Vec<AiSegment>,
        excluded: Vec<String>,
    ) {
        // 两个路径设非空：避免 load() 的「空路径补默认值」dirty 回写干扰测试
        let mut s = settings_repo::Settings {
            screenshot_path: "/tmp/hindsight-test-shots".to_string(),
            ..Default::default()
        };
        s.ai.models_path = "/tmp/hindsight-test-models".to_string();
        s.ai.external_enabled = true;
        s.ai.summary_main = SUMMARY_CLOUD_SENTINEL.to_string();
        s.ai.endpoint = format!("http://127.0.0.1:{port}/v1");
        s.ai.model = "canned-model".to_string();
        s.ai.segments = segments;
        s.ai.excluded_categories = excluded;
        settings_repo::save(pool, &s).await.unwrap();
    }

    /// 插一行已 seal 的 activities 行（device_id 可指定，测设备过滤用）。
    async fn insert_act_dev(
        pool: &DbPool,
        local_date: &str,
        local_hour: u8,
        process_name: &str,
        window_title: &str,
        duration_secs: i64,
        device_id: &str,
    ) {
        let local_date = local_date.to_string();
        let process_name = process_name.to_string();
        let window_title = window_title.to_string();
        let device_id = device_id.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(
                        ?1 || 'T' || printf('%02d', ?2) || ':00:00Z',
                        ?1 || 'T' || printf('%02d', ?2) || ':00:30Z',
                        ?3, ?1, ?2,
                        ?4, ?5, 'other', ?6,
                        ?1 || 'T' || printf('%02d', ?2) || ':00:30Z',
                        'local'
                     )",
                    rusqlite::params![
                        local_date,
                        local_hour as i64,
                        duration_secs,
                        process_name,
                        window_title,
                        device_id,
                    ],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn insert_act(
        pool: &DbPool,
        local_date: &str,
        local_hour: u8,
        process_name: &str,
        window_title: &str,
        duration_secs: i64,
    ) {
        insert_act_dev(
            pool,
            local_date,
            local_hour,
            process_name,
            window_title,
            duration_secs,
            TEST_SELF_ID,
        )
        .await;
    }

    /// 建一个「进程名 = 组名」的 app_group 并挂到指定分类——测排除分类用。
    async fn seed_solo_group(pool: &DbPool, name: &str, category_id: &str) {
        let name = name.to_string();
        let category_id = category_id.to_string();
        pool.0
            .call(move |conn| {
                let now = "2026-05-15T10:00:00Z";
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES(?1, ?1, ?2, ?3, NULL)",
                    rusqlite::params![name, category_id, now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES(?1, ?1, ?2, NULL)",
                    rusqlite::params![name, now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    fn make_runner(
        pool: &DbPool,
        sink: RecordingSink,
        cancel: Arc<AtomicBool>,
    ) -> DaySummaryRunner<RecordingSink> {
        // 云端路由下 supervisor 只是占位——ensure_summary_engine 直接短路返回，
        // summarize_segment 的 External 分支也不 acquire 它
        DaySummaryRunner::new(
            pool.clone(),
            Arc::new(EngineSupervisor::new()),
            sink,
            cancel,
        )
    }

    fn date_2026_05_15() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 15).unwrap()
    }

    /// 按 segment_idx 取行——get_day 的排序不属于本测试关心的契约。
    fn row_by_idx(
        rows: &[crate::repo::ai_summaries::SegmentSummaryRow],
        idx: u32,
    ) -> &crate::repo::ai_summaries::SegmentSummaryRow {
        rows.iter()
            .find(|r| r.segment_idx == idx)
            .unwrap_or_else(|| panic!("缺少段 {idx} 的行: {rows:?}"))
    }

    // ─────────────────────────────── 全天编排 ───────────────────────────────

    /// 全天多段循环的主干契约：有活动的段生成 ok 行、无活动的段落
    /// skipped_no_activity、时间范围非法的段静默跳过（不发事件不落行不计请求），
    /// 进度事件严格按「段 started →（有活动才有 summarizing）→ done → … → all_done」
    /// 排列，云端路由下全程没有 engine_starting。
    #[tokio::test]
    async fn run_full_day_generates_ok_and_skipped_segments_in_order() {
        let pool = fresh_test_pool().await;
        // 9 点（上午段）与 20 点（晚上段）有活动；下午段 13-18 整段空白
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        insert_act(&pool, "2026-05-15", 20, "Chrome", "docs", 300).await;

        // 两发成功脚本：恰好对应两个有活动的段；多余请求会撞 500 被断言抓住
        let (port, bodies) = spawn_scripted_openai_server(vec![
            Canned::Ok("上午写了测试。"),
            Canned::Ok("晚上看了文档。"),
        ])
        .await;
        seed_cloud_settings(
            &pool,
            port,
            vec![
                seg("上午", 9, 12),
                seg("坏段", 12, 12), // end<=start：应被静默跳过
                seg("下午", 13, 18),
                seg("晚上", 19, 23),
            ],
            vec![],
        )
        .await;

        let sink = RecordingSink::default();
        let events = sink.events.clone();
        let runner = make_runner(&pool, sink, Arc::new(AtomicBool::new(false)));
        runner
            .run("daily", date_2026_05_15(), DeviceFilter::All, false, None)
            .await
            .expect("全天编排应成功");

        // 事件序列独立推导：段 0 有活动（3 事件）→ 段 1 非法（0 事件）→
        // 段 2 无活动（started + done，无 summarizing）→ 段 3 有活动 → all_done
        // 快照(clone)而非持 guard:后续还有 await,std MutexGuard 跨 await 会触发
        // clippy::await_holding_lock;run 已返回,事件不会再变,快照语义等价。
        let ev = events.lock().unwrap().clone();
        assert_eq!(
            phase_seq(&ev),
            vec![
                ("segment_started", Some(0)),
                ("summarizing", Some(0)),
                ("segment_done", Some(0)),
                ("segment_started", Some(2)),
                ("segment_done", Some(2)),
                ("segment_started", Some(3)),
                ("summarizing", Some(3)),
                ("segment_done", Some(3)),
                ("all_done", None),
            ],
            "事件序列不符: {:?}",
            phase_seq(&ev)
        );
        // 云端路由绝不 emit engine_starting；total_segments 计入非法段（4）
        for p in ev.iter() {
            assert_eq!(p.source, "daily");
            assert_eq!(p.date, "2026-05-15");
            assert_eq!(p.total_segments, 4, "total_segments 应为段配置总数");
        }
        // 段 2 的 done 事件应带 skipped 状态 + 空正文
        let done2 = ev
            .iter()
            .find(|p| p.phase == "segment_done" && p.segment_idx == Some(2))
            .unwrap();
        assert_eq!(done2.status, Some("skipped_no_activity"));
        assert_eq!(done2.content.as_deref(), Some(""));

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 3, "非法段不落行，应恰 3 行: {rows:?}");
        let r0 = row_by_idx(&rows, 0);
        assert_eq!(r0.status, "ok");
        assert_eq!(r0.content, "上午写了测试。");
        assert_eq!(
            r0.model, "canned-model",
            "云端路由落库 model 应是云端模型 ID"
        );
        assert_eq!(r0.label, "上午");
        assert_eq!((r0.start_hour, r0.end_hour), (9, 12));
        let r2 = row_by_idx(&rows, 2);
        assert_eq!(r2.status, "skipped_no_activity");
        assert!(r2.content.is_empty() && r2.error.is_none());
        let r3 = row_by_idx(&rows, 3);
        assert_eq!(r3.status, "ok");
        assert_eq!(r3.content, "晚上看了文档。");
        // 恰发出 2 个 LLM 请求：无活动段与非法段都不许打云端
        assert_eq!(bodies.lock().unwrap().len(), 2, "无活动/非法段不应发请求");
    }

    /// 单段云端 500 不许炸整天：失败段落 status='error' 行（带可读错误），
    /// 其余段照常生成，收尾仍是 all_done、run 返回 Ok。
    #[tokio::test]
    async fn run_segment_failure_writes_error_row_and_continues() {
        let pool = fresh_test_pool().await;
        for h in [9u8, 13, 19] {
            insert_act(&pool, "2026-05-15", h, "VSCode", "a.rs", 300).await;
        }
        let (port, _bodies) = spawn_scripted_openai_server(vec![
            Canned::Ok("第一段成功。"),
            Canned::Http500,
            Canned::Ok("第三段成功。"),
        ])
        .await;
        seed_cloud_settings(
            &pool,
            port,
            vec![seg("早", 9, 12), seg("午", 13, 18), seg("晚", 19, 23)],
            vec![],
        )
        .await;

        let sink = RecordingSink::default();
        let events = sink.events.clone();
        let runner = make_runner(&pool, sink, Arc::new(AtomicBool::new(false)));
        runner
            .run("daily", date_2026_05_15(), DeviceFilter::All, false, None)
            .await
            .expect("单段失败不应让整天返回 Err");

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(row_by_idx(&rows, 0).status, "ok");
        let r1 = row_by_idx(&rows, 1);
        assert_eq!(r1.status, "error");
        assert!(r1.content.is_empty(), "失败段不应有正文: {:?}", r1.content);
        assert!(
            r1.error.as_deref().is_some_and(|e| e.contains("500")),
            "error 字段应带 500 描述: {:?}",
            r1.error
        );
        let r2 = row_by_idx(&rows, 2);
        assert_eq!(r2.status, "ok", "失败段之后的段必须继续生成");
        assert_eq!(r2.content, "第三段成功。");

        // segment_done 状态按段序应为 ok / error / ok；error 事件带 message；最后 all_done
        // 快照(clone)而非持 guard:后续还有 await,std MutexGuard 跨 await 会触发
        // clippy::await_holding_lock;run 已返回,事件不会再变,快照语义等价。
        let ev = events.lock().unwrap().clone();
        let done_statuses: Vec<_> = ev
            .iter()
            .filter(|p| p.phase == "segment_done")
            .map(|p| (p.segment_idx, p.status))
            .collect();
        assert_eq!(
            done_statuses,
            vec![
                (Some(0), Some("ok")),
                (Some(1), Some("error")),
                (Some(2), Some("ok"))
            ]
        );
        let done1 = ev
            .iter()
            .find(|p| p.segment_idx == Some(1) && p.phase == "segment_done")
            .unwrap();
        assert!(
            done1.message.as_deref().is_some_and(|m| !m.is_empty()),
            "error 段的 done 事件应携带可读 message"
        );
        assert_eq!(ev.last().unwrap().phase, "all_done");
    }

    // ─────────────────────────────── 取消语义 ───────────────────────────────

    /// 进门前就已按下停止：一个段都不许启动、不发任何 LLM 请求、不落任何行，
    /// 只 emit 一个 cancelled（segment_idx=0 表示停在第 0 段门口），run 返回 Ok。
    #[tokio::test]
    async fn run_cancel_preset_starts_nothing() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "a.rs", 300).await;
        // endpoint 指向必然拒连的端口：若 runner 违约发出请求，
        // 会以 error 行 / error 事件形式在下面的断言里暴露
        seed_cloud_settings(&pool, free_local_port(), vec![seg("早", 9, 12)], vec![]).await;

        let sink = RecordingSink::default();
        let events = sink.events.clone();
        let runner = make_runner(&pool, sink, Arc::new(AtomicBool::new(true)));
        runner
            .run("daily", date_2026_05_15(), DeviceFilter::All, false, None)
            .await
            .expect("取消是正常收尾，不是错误");

        // 快照(clone)而非持 guard:后续还有 await,std MutexGuard 跨 await 会触发
        // clippy::await_holding_lock;run 已返回,事件不会再变,快照语义等价。
        let ev = events.lock().unwrap().clone();
        assert_eq!(phase_seq(&ev), vec![("cancelled", Some(0))]);
        assert!(
            ai_summaries::get_day(&pool, "daily", "2026-05-15")
                .await
                .unwrap()
                .is_empty(),
            "取消不应留下任何行"
        );
    }

    /// 第一段完成瞬间按停止：后续段不再启动（不发请求不落行），
    /// 已完成段的成果保留，cancelled 事件的 segment_idx 指向被拦下的段。
    #[tokio::test]
    async fn run_cancel_after_first_segment_skips_rest() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "a.rs", 300).await;
        insert_act(&pool, "2026-05-15", 13, "Chrome", "b", 300).await;
        // 脚本只有一发成功：若段 1 违约发请求会拿到 500 落 error 行，被下面断言抓住
        let (port, bodies) = spawn_scripted_openai_server(vec![Canned::Ok("第一段完成。")]).await;
        seed_cloud_settings(
            &pool,
            port,
            vec![seg("早", 9, 12), seg("午", 13, 18)],
            vec![],
        )
        .await;

        let cancel = Arc::new(AtomicBool::new(false));
        let sink = RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
            cancel_on_segment_done: Some(cancel.clone()),
        };
        let events = sink.events.clone();
        let runner = make_runner(&pool, sink, cancel);
        runner
            .run("daily", date_2026_05_15(), DeviceFilter::All, false, None)
            .await
            .unwrap();

        // 快照(clone)而非持 guard:后续还有 await,std MutexGuard 跨 await 会触发
        // clippy::await_holding_lock;run 已返回,事件不会再变,快照语义等价。
        let ev = events.lock().unwrap().clone();
        assert_eq!(
            phase_seq(&ev),
            vec![
                ("segment_started", Some(0)),
                ("summarizing", Some(0)),
                ("segment_done", Some(0)),
                ("cancelled", Some(1)), // 停在第 1 段门口，没有 all_done
            ]
        );
        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "只应有已完成段的行: {rows:?}");
        assert_eq!(row_by_idx(&rows, 0).status, "ok");
        assert_eq!(bodies.lock().unwrap().len(), 1, "被取消的段不应发请求");
    }

    /// 在途请求中按停止（假服务收下连接后永不回包）：cancellable 的 250ms
    /// 轮询要能打断请求，该段**不落行**（下次生成自然重跑），run 以
    /// cancelled 事件优雅收尾并返回 Ok。
    #[tokio::test]
    async fn run_cancel_during_inflight_request_leaves_no_row() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "a.rs", 300).await;
        // 挂起的假服务：backlog 完成 TCP 握手但应用层永不 accept/回包
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        seed_cloud_settings(&pool, port, vec![seg("早", 9, 12)], vec![]).await;

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = cancel.clone();
        // 100ms 后按停止：早于 cancellable 的第一次 250ms 轮询，请求必然还在途
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            cancel_bg.store(true, Ordering::Relaxed);
        });

        let sink = RecordingSink::default();
        let events = sink.events.clone();
        let runner = make_runner(&pool, sink, cancel);
        runner
            .run("daily", date_2026_05_15(), DeviceFilter::All, false, None)
            .await
            .expect("在途取消应优雅收尾返回 Ok");

        // 快照(clone)而非持 guard:后续还有 await,std MutexGuard 跨 await 会触发
        // clippy::await_holding_lock;run 已返回,事件不会再变,快照语义等价。
        let ev = events.lock().unwrap().clone();
        assert_eq!(
            phase_seq(&ev),
            vec![
                ("segment_started", Some(0)),
                ("summarizing", Some(0)),
                // 深处抛 SummaryCancelled 由 run() 统一收尾：cancelled 不带段号
                ("cancelled", None),
            ]
        );
        assert!(
            ai_summaries::get_day(&pool, "daily", "2026-05-15")
                .await
                .unwrap()
                .is_empty(),
            "在途取消的段不应落行"
        );
        drop(listener);
    }

    // ──────────────────────── 复用 / 强刷 / 配置校验 ────────────────────────

    /// force_refresh=false 时已 ok 的段直接复用：不发请求、不发段事件、
    /// 行内容原样保留；force_refresh=true 则清空当天重新生成。
    #[tokio::test]
    async fn run_reuses_ok_segments_and_force_refresh_regenerates() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "a.rs", 300).await;

        // 第一轮：正常生成
        let (port1, bodies1) = spawn_scripted_openai_server(vec![Canned::Ok("第一版总结。")]).await;
        seed_cloud_settings(&pool, port1, vec![seg("早", 9, 12)], vec![]).await;
        let runner = make_runner(
            &pool,
            RecordingSink::default(),
            Arc::new(AtomicBool::new(false)),
        );
        runner
            .run("daily", date_2026_05_15(), DeviceFilter::All, false, None)
            .await
            .unwrap();
        assert_eq!(bodies1.lock().unwrap().len(), 1);

        // 第二轮 force_refresh=false：脚本已耗尽（再请求必 500 覆盖成 error 行），
        // 复用正确则行保持第一版、事件只有 all_done
        let sink2 = RecordingSink::default();
        let events2 = sink2.events.clone();
        let runner2 = make_runner(&pool, sink2, Arc::new(AtomicBool::new(false)));
        runner2
            .run("daily", date_2026_05_15(), DeviceFilter::All, false, None)
            .await
            .unwrap();
        assert_eq!(
            phase_seq(&events2.lock().unwrap()),
            vec![("all_done", None)],
            "复用段不应发任何段级事件"
        );
        assert_eq!(bodies1.lock().unwrap().len(), 1, "复用不应发新请求");
        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].content, "第一版总结。", "复用应保留原行");

        // 第三轮 force_refresh=true：换新假服务，应清空重新生成出第二版
        let (port3, bodies3) = spawn_scripted_openai_server(vec![Canned::Ok("第二版总结。")]).await;
        seed_cloud_settings(&pool, port3, vec![seg("早", 9, 12)], vec![]).await;
        let runner3 = make_runner(
            &pool,
            RecordingSink::default(),
            Arc::new(AtomicBool::new(false)),
        );
        runner3
            .run("daily", date_2026_05_15(), DeviceFilter::All, true, None)
            .await
            .unwrap();
        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "强刷后仍只应一行: {rows:?}");
        assert_eq!(rows[0].content, "第二版总结。", "强刷应覆盖成新内容");
        assert_eq!(bodies3.lock().unwrap().len(), 1);
    }

    /// 既没选本地模型也没选云端：进门即报 InvalidInput（前端错误条引导去配置），
    /// 不发任何事件不落任何行。
    #[tokio::test]
    async fn run_without_model_or_cloud_errors_before_any_work() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "a.rs", 300).await;
        // 默认 settings：external_enabled=false 且 summary/active main 全空
        let mut s = settings_repo::Settings {
            screenshot_path: "/tmp/hindsight-test-shots".to_string(),
            ..Default::default()
        };
        s.ai.models_path = "/tmp/hindsight-test-models".to_string();
        settings_repo::save(&pool, &s).await.unwrap();

        let sink = RecordingSink::default();
        let events = sink.events.clone();
        let runner = make_runner(&pool, sink, Arc::new(AtomicBool::new(false)));
        let r = runner
            .run("daily", date_2026_05_15(), DeviceFilter::All, false, None)
            .await;
        assert!(
            matches!(r, Err(Error::InvalidInput(_))),
            "无模型无云端应报 InvalidInput: {r:?}"
        );
        assert!(events.lock().unwrap().is_empty(), "配置校验失败不应发事件");
        assert!(ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap()
            .is_empty());
    }

    // ────────────────────── 材料过滤在 runner 层的传递 ──────────────────────

    /// excluded_categories 与 DeviceFilter 必须从 settings / 参数一路传到
    /// 材料合成：被排除分类的活动与他机活动都不许进 prompt；某段若只剩
    /// 被排除的活动，该段应落 skipped_no_activity（而不是拿空材料去骗模型）。
    #[tokio::test]
    async fn run_passes_excluded_categories_and_device_filter_to_materials() {
        let pool = fresh_test_pool().await;
        seed_solo_group(&pool, "VSCode", "code").await;
        seed_solo_group(&pool, "Slack", "browse").await;
        // 段 0（9-10 点）：本机 VSCode 300s + 本机 Slack(被排除) + 他机 VSCode 6000s
        insert_act(&pool, "2026-05-15", 9, "VSCode", "main.rs", 300).await;
        insert_act(&pool, "2026-05-15", 9, "Slack", "chat", 300).await;
        insert_act_dev(
            &pool,
            "2026-05-15",
            9,
            "VSCode",
            "other.rs",
            6000,
            "other-device",
        )
        .await;
        // 段 1（13-14 点）：只有被排除分类的活动 → 过滤后整段无材料
        insert_act(&pool, "2026-05-15", 13, "Slack", "salon", 300).await;

        let (port, bodies) =
            spawn_scripted_openai_server(vec![Canned::Ok("只统计了本机代码活动。")]).await;
        seed_cloud_settings(
            &pool,
            port,
            vec![seg("早", 9, 10), seg("午", 13, 14)],
            vec!["browse".to_string()],
        )
        .await;

        let runner = make_runner(
            &pool,
            RecordingSink::default(),
            Arc::new(AtomicBool::new(false)),
        );
        runner
            .run(
                "daily",
                date_2026_05_15(),
                DeviceFilter::Only(TEST_SELF_ID.to_string()),
                false,
                None,
            )
            .await
            .unwrap();

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(row_by_idx(&rows, 0).status, "ok");
        assert_eq!(
            row_by_idx(&rows, 1).status,
            "skipped_no_activity",
            "只剩被排除活动的段应判定为无活动"
        );

        // 段 0 的请求 body（prompt 材料）：本机未排除活动在场，
        // 被排除分类与他机活动的一切痕迹（app 名 / 标题 / 时长放大）都不在场
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1, "只有段 0 应发请求");
        let b = &bodies[0];
        assert!(
            b.contains("VSCode 5 分钟"),
            "本机 300s 应呈现为 5 分钟: {b}"
        );
        assert!(b.contains("main.rs"), "本机窗口标题应进材料: {b}");
        assert!(!b.contains("Slack"), "被排除分类的 app 不应进材料: {b}");
        assert!(!b.contains("other.rs"), "他机窗口标题不应进材料: {b}");
        assert!(!b.contains("105 分钟"), "他机时长不应混入聚合: {b}");
    }

    // ─────────────────────────── 单段重试入口 ───────────────────────────

    /// run_one_segment_only 只跑指定段：其它段既不生成也不发事件，
    /// 且**不** emit all_done（它不是"整天完成"）。
    #[tokio::test]
    async fn run_one_segment_only_touches_only_that_segment() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "a.rs", 300).await;
        insert_act(&pool, "2026-05-15", 13, "Chrome", "b", 300).await;
        let (port, bodies) =
            spawn_scripted_openai_server(vec![Canned::Ok("只重试了下午段。")]).await;
        seed_cloud_settings(
            &pool,
            port,
            vec![seg("早", 9, 12), seg("午", 13, 18)],
            vec![],
        )
        .await;

        let sink = RecordingSink::default();
        let events = sink.events.clone();
        let runner = make_runner(&pool, sink, Arc::new(AtomicBool::new(false)));
        runner
            .run_one_segment_only("daily", date_2026_05_15(), 1, DeviceFilter::All, None)
            .await
            .unwrap();

        // 快照(clone)而非持 guard:后续还有 await,std MutexGuard 跨 await 会触发
        // clippy::await_holding_lock;run 已返回,事件不会再变,快照语义等价。
        let ev = events.lock().unwrap().clone();
        assert_eq!(
            phase_seq(&ev),
            vec![
                ("segment_started", Some(1)),
                ("summarizing", Some(1)),
                ("segment_done", Some(1)),
            ],
            "单段重试不应有 all_done / 其它段事件"
        );
        // total_segments 仍按整天段数报（前端进度条口径）
        assert!(ev.iter().all(|p| p.total_segments == 2));
        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "只应写指定段的行: {rows:?}");
        assert_eq!(row_by_idx(&rows, 1).content, "只重试了下午段。");
        assert_eq!(bodies.lock().unwrap().len(), 1);
    }

    /// 单段重试的入参校验：段下标越界报 InvalidInputDyn，
    /// 指到时间范围非法的段报 InvalidInput——都不落行不发段事件。
    #[tokio::test]
    async fn run_one_segment_only_rejects_bad_index_and_degenerate_segment() {
        let pool = fresh_test_pool().await;
        let (port, bodies) = spawn_scripted_openai_server(vec![]).await;
        seed_cloud_settings(
            &pool,
            port,
            vec![seg("早", 9, 12), seg("坏", 12, 12)],
            vec![],
        )
        .await;

        let sink = RecordingSink::default();
        let events = sink.events.clone();
        let runner = make_runner(&pool, sink, Arc::new(AtomicBool::new(false)));

        let r = runner
            .run_one_segment_only("daily", date_2026_05_15(), 5, DeviceFilter::All, None)
            .await;
        assert!(
            matches!(r, Err(Error::InvalidInputDyn(_))),
            "越界下标应报 InvalidInputDyn: {r:?}"
        );

        let r = runner
            .run_one_segment_only("daily", date_2026_05_15(), 1, DeviceFilter::All, None)
            .await;
        assert!(
            matches!(r, Err(Error::InvalidInput(_))),
            "非法时间范围的段应报 InvalidInput: {r:?}"
        );

        assert!(events.lock().unwrap().is_empty(), "校验失败不应发任何事件");
        assert!(ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(bodies.lock().unwrap().len(), 0, "校验失败不应发请求");
    }

    // ──────────────────── 本地引擎路径（不依赖真 llama-server 的部分） ────────────────────

    /// 本地路由选了模型但 GGUF 文件已不存在（被删 / 移动 / 换盘）：
    /// 应先 emit engine_starting（引擎冷启动提示），随后在解析模型路径时报
    /// ModelFileMissing 整轮失败——绝不能落半截段行。真正 spawn llama-server
    /// 之前就拦下，所以这条分支不需要真引擎也能测。
    #[tokio::test]
    async fn run_local_route_missing_model_file_errors_after_engine_starting() {
        let pool = fresh_test_pool().await;
        insert_act(&pool, "2026-05-15", 9, "VSCode", "a.rs", 300).await;

        let mut s = settings_repo::Settings {
            screenshot_path: "/tmp/hindsight-test-shots".to_string(),
            ..Default::default()
        };
        // models_path 指向必然不存在 GGUF 的空目录路径；选了本地模型名 → 走本地引擎分支
        s.ai.models_path = "/tmp/hindsight-test-models-nonexistent".to_string();
        s.ai.summary_main = "ghost.gguf".to_string();
        s.ai.segments = vec![seg("早", 9, 12)];
        settings_repo::save(&pool, &s).await.unwrap();

        let sink = RecordingSink::default();
        let events = sink.events.clone();
        let runner = make_runner(&pool, sink, Arc::new(AtomicBool::new(false)));
        let r = runner
            .run("daily", date_2026_05_15(), DeviceFilter::All, false, None)
            .await;
        assert!(
            matches!(r, Err(Error::ModelFileMissing(_))),
            "模型文件缺失应报 ModelFileMissing: {r:?}"
        );

        // 快照(clone)而非持 guard:后续还有 await,std MutexGuard 跨 await 会触发
        // clippy::await_holding_lock;run 已返回,事件不会再变,快照语义等价。
        let ev = events.lock().unwrap().clone();
        assert_eq!(
            phase_seq(&ev),
            vec![("engine_starting", None)],
            "应先提示引擎启动、失败后无任何段事件"
        );
        // message 契约：留 None 让前端按 phase 显示本地化文案
        assert!(ev[0].message.is_none());
        assert!(
            ai_summaries::get_day(&pool, "daily", "2026-05-15")
                .await
                .unwrap()
                .is_empty(),
            "引擎启动失败不应留下任何段行"
        );
    }
}
