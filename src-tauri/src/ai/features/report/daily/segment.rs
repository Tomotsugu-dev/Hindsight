//! 日报一段的结果写库：段总结，以及没有活动时的占位行。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::ai::config::AiConfig;
use crate::ai::features::report::progress::cancellable;
use crate::ai::llm::Step2Chat;
use crate::ai::prompt::{build_system_prompt, build_user_prompt, SegmentContext};
use crate::error::{Error, Result};
use crate::repo::ai_summaries::{self, SegmentSummaryRow};
use crate::storage::{utc_now_rfc3339, DbPool};

/// `ai_summaries` 中单个时段行的定位信息与时间范围。
pub(super) struct SegmentTarget<'a> {
    pub source: &'a str,
    pub date: &'a str,
    pub index: u32,
    pub label: &'a str,
    pub start_hour: u8,
    pub end_hour: u8,
}

/// 段总结：拿活动时间线 + top_apps 拼 prompt → 调 LLM → 落库。
///
/// 落库语义：
/// - chat 成功 → status = "ok"
/// - chat 失败 → status = "error"，error 字段塞错误描述（不抛 Err，让上层继续走）
/// - DB 写入失败 → 抛 Err，整段失败
///
/// 返回 `(已落库的行, status_str)`，让调用方拼 `segment_done` 事件 payload 用。
pub(super) async fn summarize_segment(
    pool: &DbPool,
    model: &Step2Chat,
    ai: &AiConfig,
    target: &SegmentTarget<'_>,
    timeline: &[(String, String)],
    top_apps: &[(String, u32, String)],
    // 停止按钮的取消标志：置位时中断在途请求，向上抛 SummaryCancelled
    //（**不写行**——该段下次生成自然重跑），由 runner 统一 emit cancelled 收尾
    cancel: &Arc<AtomicBool>,
) -> Result<(SegmentSummaryRow, &'static str)> {
    let ctx = SegmentContext {
        label: target.label,
        start_hour: target.start_hour,
        end_hour: target.end_hour,
        top_apps,
        timeline,
    };
    let system = build_system_prompt(ai);
    let user_text = build_user_prompt(ai, &ctx);

    let chat_result = cancellable(cancel, model.chat(&system, &user_text)).await;
    if matches!(chat_result, Err(Error::SummaryCancelled)) {
        return Err(Error::SummaryCancelled);
    }

    let model_label = model.model_label().to_string();
    let (row, status_str): (SegmentSummaryRow, &'static str) = match chat_result {
        Ok(content) => (
            SegmentSummaryRow {
                source: target.source.to_string(),
                local_date: target.date.to_string(),
                segment_idx: target.index,
                label: target.label.to_string(),
                start_hour: target.start_hour,
                end_hour: target.end_hour,
                content,
                model: model_label,
                status: "ok".to_string(),
                error: None,
                generated_at: utc_now_rfc3339(),
            },
            "ok",
        ),
        Err(e) => (
            SegmentSummaryRow {
                source: target.source.to_string(),
                local_date: target.date.to_string(),
                segment_idx: target.index,
                label: target.label.to_string(),
                start_hour: target.start_hour,
                end_hour: target.end_hour,
                content: String::new(),
                model: model_label,
                status: "error".to_string(),
                error: Some(e.to_string()),
                generated_at: utc_now_rfc3339(),
            },
            "error",
        ),
    };

    // upsert 失败不让整轮 daily 抛飞——磁盘满 / DB lock 时 row 写不进去也得让上层
    // emit segment_done 把当前 row 推给前端（至少能看到红色 error badge + 错误描述）。
    if let Err(e) = ai_summaries::upsert_segment(pool, &row).await {
        log::error!(
            "ai_summaries upsert 失败（段 {} status={}）：{e}",
            row.segment_idx,
            row.status,
        );
    }
    Ok((row, status_str))
}

/// 让某段直接落 `skipped_no_activity` 行 —— 该段完全没有活动记录时的兜底。
pub(super) async fn write_skipped_segment(
    pool: &DbPool,
    target: &SegmentTarget<'_>,
    model_label: String,
) -> Result<()> {
    ai_summaries::upsert_segment(
        pool,
        &SegmentSummaryRow {
            source: target.source.to_string(),
            local_date: target.date.to_string(),
            segment_idx: target.index,
            label: target.label.to_string(),
            start_hour: target.start_hour,
            end_hour: target.end_hour,
            content: String::new(),
            model: model_label,
            status: "skipped_no_activity".to_string(),
            error: None,
            generated_at: utc_now_rfc3339(),
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::config::{ModelRef, Task};
    use crate::ai::features::report::daily::timeline::build_activity_timeline;
    use crate::ai::features::report::model::open_model;
    use crate::ai::model::Reasoning;
    use crate::ai::policy::{AiDataPolicy, AiModule, ProfileChoice};
    use crate::repo::reports::DeviceFilter;
    use crate::repo::test_util::fresh_test_pool;

    fn target<'a>(
        source: &'a str,
        date: &'a str,
        index: u32,
        label: &'a str,
        start_hour: u8,
        end_hour: u8,
    ) -> SegmentTarget<'a> {
        SegmentTarget {
            source,
            date,
            index,
            label,
            start_hour,
            end_hour,
        }
    }

    /// 端到端:真实主库 → 新单步管线(活动时间线 + 云端文本模型)→ 写回 daily 日报。
    /// 跑法:
    ///   `DAILY_E2E_DATE=2026-07-05 CHAT_E2E_ENDPOINT=... CHAT_E2E_MODEL=... CHAT_E2E_KEY=... \
    ///    cargo test --lib daily::segment::tests::e2e -- --ignored --nocapture`
    /// 写的是真实 ai_summaries(source='daily'),会先清掉当天旧行。
    #[tokio::test]
    #[ignore]
    async fn e2e_regenerate_daily_report() {
        let date = std::env::var("DAILY_E2E_DATE").expect("设 DAILY_E2E_DATE=YYYY-MM-DD");
        let endpoint = std::env::var("CHAT_E2E_ENDPOINT").expect("设 CHAT_E2E_ENDPOINT");
        let model = std::env::var("CHAT_E2E_MODEL").expect("设 CHAT_E2E_MODEL");
        let api_key = std::env::var("CHAT_E2E_KEY").unwrap_or_default();

        let pool = DbPool::open(&crate::storage::db_path().unwrap())
            .await
            .unwrap();
        let cfg = crate::repo::settings::load(&pool).await.unwrap();
        // 强制云端文本路由(用传入凭据),其余(段划分/排除分类/语言/简介)沿用用户设置
        let mut ai = cfg.ai.clone();
        ai.cloud = vec![crate::ai::config::CloudConnection {
            id: "e2e".to_string(),
            name: "e2e".to_string(),
            dialect: "openai".to_string(),
            endpoint,
            model,
            api_key,
            context_length: None,
        }];
        ai.picks.daily = Some(ModelRef::Cloud {
            connection: "e2e".to_string(),
        });

        ai_summaries::clear_day(&pool, "daily", &date)
            .await
            .unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let chat_model = open_model(&ai, Task::Daily, 0).unwrap();
        let visible = AiDataPolicy::load(&pool.0, AiModule::Daily, &ProfileChoice::Auto)
            .await
            .unwrap()
            .main_filter();

        for (idx, seg) in ai.segments.iter().enumerate() {
            if seg.end_hour <= seg.start_hour {
                continue;
            }
            let timeline = build_activity_timeline(
                &pool,
                &date,
                seg.start_hour,
                seg.end_hour,
                &visible,
                &DeviceFilter::All,
            )
            .await
            .unwrap();
            println!(
                "\n===== 段 {idx} {}({:02}:00-{:02}:00) 时间线 {} 行",
                seg.label,
                seg.start_hour,
                seg.end_hour,
                timeline.len()
            );
            let slot = target(
                "daily",
                &date,
                idx as u32,
                &seg.label,
                seg.start_hour,
                seg.end_hour,
            );
            if timeline.is_empty() {
                write_skipped_segment(&pool, &slot, chat_model.model_label().to_string())
                    .await
                    .unwrap();
                println!("(无活动,skipped)");
                continue;
            }
            let top_apps = crate::repo::ai_summaries::list_segment_top_apps(
                &pool,
                &date,
                seg.start_hour,
                seg.end_hour,
                &visible,
                DeviceFilter::All,
                8,
            )
            .await
            .unwrap_or_default();
            let (row, status) = summarize_segment(
                &pool,
                &chat_model,
                &ai,
                &slot,
                &timeline,
                &top_apps,
                &cancel,
            )
            .await
            .unwrap();
            println!("[{status}]\n{}", row.content);
            if status == "error" {
                println!("error: {:?}", row.error);
            }
        }
    }

    /// 拿一个刚刚释放的本地端口——连接必然被拒，用来模拟"引擎没起来/端口配错"。
    fn free_local_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    /// 起一个只回一发 canned OpenAI 兼容响应的本地假服务，返回端口。
    /// 读完整个请求（headers + Content-Length body）再回包，避免半途关闭触发 RST。
    async fn spawn_canned_openai_server(content: &str) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = serde_json::json!({
            "choices": [{
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 42, "completion_tokens": 17 }
        })
        .to_string();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = sock.read(&mut tmp).await.unwrap();
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
                        break;
                    }
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            let _ = sock.shutdown().await;
        });
        port
    }

    /// 重跑同段必须覆盖不叠加——PK (source, date, idx) 语义回归：
    /// 若重复插行，前端日报同一段会渲染两次。
    #[tokio::test]
    async fn skipped_row_upsert_overwrites_not_duplicates() {
        let pool = fresh_test_pool().await;
        let first = target("daily", "2026-05-15", 0, "上午", 9, 12);
        write_skipped_segment(&pool, &first, "m1".into())
            .await
            .unwrap();
        // 用户改了段配置后重跑：label / 时段 / 模型都变，应整行覆盖
        let second = target("daily", "2026-05-15", 0, "早晨", 8, 12);
        write_skipped_segment(&pool, &second, "m2".into())
            .await
            .unwrap();

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "重跑应覆盖不叠加: {rows:?}");
        let r = &rows[0];
        assert_eq!(r.status, "skipped_no_activity");
        assert_eq!(r.label, "早晨", "第二次的字段应覆盖第一次");
        assert_eq!(r.start_hour, 8);
        assert_eq!(r.model, "m2");
        assert!(r.content.is_empty() && r.error.is_none());
    }

    /// write_skipped_segment 是"整段无活动"的兜底落库路径。
    /// 真实翻车场景：兜底行 status 拼错或 content 不为空，前端会把空白段
    /// 当成正常总结渲染出一张空卡片；覆盖旧 error 行失败则重跑后错误提示不消失。
    #[tokio::test]
    async fn write_skipped_segment_writes_placeholder_and_overwrites() {
        let pool = fresh_test_pool().await;
        let date = "2026-07-20";

        // 先落一行 error（模拟前一轮 LLM 失败），兜底必须能覆盖它
        let bad = SegmentSummaryRow {
            source: "daily".to_string(),
            local_date: date.to_string(),
            segment_idx: 3,
            label: "段3".to_string(),
            start_hour: 9,
            end_hour: 12,
            content: "旧的总结正文".to_string(),
            model: "test-model.gguf".to_string(),
            status: "error".to_string(),
            error: Some("boom".to_string()),
            generated_at: "2026-07-20T01:02:03+00:00".to_string(),
        };
        ai_summaries::upsert_segment(&pool, &bad).await.unwrap();

        let slot = target("daily", date, 3, "下午", 13, 18);
        write_skipped_segment(&pool, &slot, "test-model.gguf".to_string())
            .await
            .unwrap();

        let rows = ai_summaries::get_day(&pool, "daily", date).await.unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.status, "skipped_no_activity");
        assert_eq!(r.content, "", "兜底行不该有正文");
        assert_eq!(r.error, None, "兜底行必须清掉旧 error");
        assert_eq!(r.label, "下午");
        assert_eq!((r.start_hour, r.end_hour), (13, 18));
        assert!(
            chrono::DateTime::parse_from_rfc3339(&r.generated_at).is_ok(),
            "generated_at 应自动填合法时间戳"
        );
    }

    /// 用户对从没生成过的日期点"强制刷新"：空库删除必须静默成功，不能报错。
    #[tokio::test]
    async fn clear_day_on_empty_db_is_ok() {
        let pool = fresh_test_pool().await;
        ai_summaries::clear_day(&pool, "daily", "2026-05-15")
            .await
            .expect("空库 clear_day 不应报错");
        assert!(ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap()
            .is_empty());
    }

    /// force_refresh 清某天日报时误伤别的 source 或相邻日期会让用户丢历史报告——
    /// DELETE 少写一个 WHERE 条件就是这个后果。
    #[tokio::test]
    async fn clear_day_spares_other_source_and_date() {
        let pool = fresh_test_pool().await;
        write_skipped_segment(
            &pool,
            &target("daily", "2026-05-15", 0, "上午", 9, 12),
            "m".into(),
        )
        .await
        .unwrap();
        write_skipped_segment(
            &pool,
            &target("daily", "2026-05-16", 0, "上午", 9, 12),
            "m".into(),
        )
        .await
        .unwrap();
        write_skipped_segment(
            &pool,
            &target("weekly", "2026-05-15", 0, "上午", 9, 12),
            "m".into(),
        )
        .await
        .unwrap();

        ai_summaries::clear_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();

        assert!(
            ai_summaries::get_day(&pool, "daily", "2026-05-15")
                .await
                .unwrap()
                .is_empty(),
            "目标 source+日期应被清空"
        );
        assert_eq!(
            ai_summaries::get_day(&pool, "daily", "2026-05-16")
                .await
                .unwrap()
                .len(),
            1,
            "相邻日期不应误伤"
        );
        assert_eq!(
            ai_summaries::get_day(&pool, "weekly", "2026-05-15")
                .await
                .unwrap()
                .len(),
            1,
            "别的 source 不应误伤"
        );
    }

    /// 引擎没起来/端口配错时 chat 必然失败：该段必须落 status='error' 行并继续
    ///（前端红色 badge 的数据源），而不是抛 Err 让整轮 daily 中断。
    #[tokio::test]
    async fn summarize_segment_chat_failure_writes_error_row() {
        let pool = fresh_test_pool().await;
        let port = free_local_port();
        let chat_model = Step2Chat::local(port, "dead.gguf", 1024, Reasoning::Default).unwrap();
        let ai = AiConfig::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let timeline = vec![("09:00-10:00".to_string(), "VSCode 30 分钟".to_string())];
        let slot = target("daily", "2026-05-15", 0, "上午", 9, 12);

        let (row, status) =
            summarize_segment(&pool, &chat_model, &ai, &slot, &timeline, &[], &cancel)
                .await
                .expect("chat 失败不应让 summarize_segment 抛 Err");

        assert_eq!(status, "error");
        assert_eq!(row.status, "error");
        assert!(row.content.is_empty(), "失败段不应有正文");
        assert!(
            row.error.as_deref().is_some_and(|e| !e.is_empty()),
            "error 字段应带可读描述: {:?}",
            row.error
        );

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "error 行必须已落库: {rows:?}");
        assert_eq!(rows[0].status, "error");
        assert_eq!(rows[0].model, "dead.gguf", "model 应取模型的 label");
    }

    /// 幂等重跑：昨天生成时该段无活动落了 skipped，今天重跑（此处走失败路径）
    /// 必须覆盖同一行——重复跑多少次都只有一行，且状态是最后一次的。
    #[tokio::test]
    async fn summarize_segment_rerun_overwrites_previous_row() {
        let pool = fresh_test_pool().await;
        let slot = target("daily", "2026-05-15", 0, "上午", 9, 12);
        write_skipped_segment(&pool, &slot, "m".into())
            .await
            .unwrap();

        let port = free_local_port();
        let chat_model = Step2Chat::local(port, "dead.gguf", 1024, Reasoning::Default).unwrap();
        let ai = AiConfig::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let timeline = vec![("09:00-10:00".to_string(), "VSCode 30 分钟".to_string())];

        for _ in 0..2 {
            summarize_segment(&pool, &chat_model, &ai, &slot, &timeline, &[], &cancel)
                .await
                .unwrap();
        }

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "skipped + 两次重跑仍应只有一行: {rows:?}");
        assert_eq!(rows[0].status, "error", "最后一次运行的状态应覆盖 skipped");
    }

    /// 停止按钮语义：取消**不落行**——该段下次生成自然重跑。若取消也写 error 行，
    /// 用户按一次停止就会看到一排假错误段。
    #[tokio::test]
    async fn summarize_segment_cancelled_writes_no_row() {
        let pool = fresh_test_pool().await;
        // 挂起的假服务：backlog 完成握手但永不回包，让请求停在途中等 cancel 打断
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let chat_model = Step2Chat::local(port, "m.gguf", 1024, Reasoning::Default).unwrap();
        let ai = AiConfig::default();
        let cancel = Arc::new(AtomicBool::new(true)); // 预先按下停止
        let timeline = vec![("09:00-10:00".to_string(), "VSCode 30 分钟".to_string())];
        let slot = target("daily", "2026-05-15", 0, "上午", 9, 12);

        let r = summarize_segment(&pool, &chat_model, &ai, &slot, &timeline, &[], &cancel).await;
        assert!(
            matches!(r, Err(Error::SummaryCancelled)),
            "取消应抛 SummaryCancelled: {r:?}"
        );
        assert!(
            ai_summaries::get_day(&pool, "daily", "2026-05-15")
                .await
                .unwrap()
                .is_empty(),
            "取消不应留下任何行"
        );
        drop(listener);
    }

    /// chat 成功的主干路径：status='ok' 行落库、正文取 choices[0] 并 trim、
    /// model 字段用模型的 label——这是日报正常生成的全部落库契约。
    #[tokio::test]
    async fn summarize_segment_ok_writes_ok_row_and_trims_content() {
        let pool = fresh_test_pool().await;
        let port = spawn_canned_openai_server("  上午主要在写 Hindsight 的单元测试。  ").await;
        let chat_model = Step2Chat::local(port, "qwen.gguf", 1024, Reasoning::Default).unwrap();
        let ai = AiConfig::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let timeline = vec![("09:00-10:00".to_string(), "VSCode 30 分钟".to_string())];
        let top_apps = vec![("VSCode".to_string(), 30u32, "code".to_string())];
        let slot = target("daily", "2026-05-15", 2, "上午", 9, 12);

        let (row, status) = summarize_segment(
            &pool,
            &chat_model,
            &ai,
            &slot,
            &timeline,
            &top_apps,
            &cancel,
        )
        .await
        .unwrap();

        assert_eq!(status, "ok");
        assert_eq!(row.status, "ok");
        assert_eq!(
            row.content, "上午主要在写 Hindsight 的单元测试。",
            "正文应取自响应且去掉首尾空白"
        );
        assert!(row.error.is_none());

        let rows = ai_summaries::get_day(&pool, "daily", "2026-05-15")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "ok 行必须已落库: {rows:?}");
        assert_eq!(rows[0].segment_idx, 2);
        assert_eq!(rows[0].status, "ok");
        assert_eq!(rows[0].content, row.content);
        assert_eq!(rows[0].model, "qwen.gguf");
    }
}
