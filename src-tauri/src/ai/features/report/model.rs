//! 按设置打开日报、周报的模型，并在走本机模型时租用 `llama-server` 引擎。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::ai::config::{AiConfig, ModelRef, Task, CLOUD_UNAVAILABLE_ERROR, NO_MODEL_ERROR};
use crate::ai::engine::{EngineGuard, EngineStartOverrides, EngineState, EngineSupervisor};
use crate::ai::features::report::progress::{cancellable, ProgressSink, SummaryProgress};
use crate::ai::llm::Step2Chat;
use crate::ai::models;
use crate::error::{Error, Result};

/// 发送 `engine_starting` 进度事件时用到的上下文。
pub(crate) struct EngineProgress<'a> {
    pub source: &'a str,
    pub date: &'a str,
    pub total_segments: u32,
}

/// 为 `task` 租用本机 `llama-server` 引擎；若配置的是云端模型则返回 `Ok(None)`。
///
/// 引擎未处于运行态时会先发 `engine_starting`；加载期间按下停止也会及时中断。
pub(crate) async fn acquire_engine<S: ProgressSink>(
    supervisor: &Arc<EngineSupervisor>,
    sink: &S,
    cancel: &Arc<AtomicBool>,
    ai: &AiConfig,
    task: Task,
    progress: EngineProgress<'_>,
) -> Result<Option<EngineGuard>> {
    let model_file = match ai.model_for(task) {
        Some(ModelRef::Cloud { .. }) => return Ok(None),
        Some(ModelRef::Local { file }) => file,
        None => return Err(Error::InvalidInput(NO_MODEL_ERROR)),
    };

    let status = supervisor.status().await;
    if status.state != EngineState::Running {
        // message 留空：前端按 phase 显示本地化的「加载模型中…」文案
        sink.emit_progress(SummaryProgress::base(
            progress.source.to_string(),
            progress.date.to_string(),
            "engine_starting",
            progress.total_segments,
        ));
    }

    let model_path = models::root_dir(ai).join(&model_file);
    if !model_path.exists() {
        return Err(Error::ModelFileMissing(format!(
            "{}（可能被删除或路径变了）",
            model_file
        )));
    }

    let overrides = EngineStartOverrides {
        batch_size: ai.batch_size_for(task),
        parallel_slots: ai.parallel_slots_for(task),
        ctx_size: ai.ctx_size_for(task),
    };
    let engine = cancellable(cancel, supervisor.acquire(model_path, overrides)).await?;
    Ok(Some(engine))
}

/// Opens the model configured for a daily or weekly report with its reasoning level.
///
/// When using a local model, `local_port` is the port of the running llama-server instance.
pub(crate) fn open_model(ai: &AiConfig, task: Task, local_port: u16) -> Result<Step2Chat> {
    let max_tokens = ai.summary_max_tokens(task);
    let reasoning = ai.reasoning_for(task);
    match ai.model_for(task) {
        Some(ModelRef::Cloud { connection }) => {
            let conn = ai
                .cloud_connection(&connection)
                .ok_or(Error::InvalidInput(CLOUD_UNAVAILABLE_ERROR))?;
            Step2Chat::cloud(
                &conn.endpoint,
                &conn.model,
                &conn.api_key,
                &conn.dialect,
                max_tokens,
                reasoning,
            )
        }
        Some(ModelRef::Local { file }) => {
            Step2Chat::local(local_port, &file, max_tokens, reasoning)
        }
        None => Err(Error::InvalidInput(NO_MODEL_ERROR)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_missing_model_routes() {
        let mut ai = AiConfig::default();
        assert!(matches!(
            open_model(&ai, Task::Daily, 8080),
            Err(Error::InvalidInput(_))
        ));

        ai.picks.daily = Some(ModelRef::Local {
            file: "qwen.gguf".to_string(),
        });
        let s = open_model(&ai, Task::Daily, 8080).unwrap();
        assert!(s.is_local());
        assert_eq!(s.model_label(), "qwen.gguf");
    }

    #[tokio::test]
    async fn reports_send_their_reasoning() {
        use crate::ai::config::{Advanced, ReasoningLevel};
        use crate::ai::transport::fake::{FakeServer, Reply};
        let server = FakeServer::start(vec![Reply::json(
            200,
            serde_json::json!({"choices": [{"message": {"role": "assistant", "content": "好"}}]}),
        )])
        .await;
        let mut ai = AiConfig::default();
        ai.picks.daily = Some(ModelRef::Local {
            file: "m.gguf".into(),
        });
        ai.advanced = vec![Advanced {
            task: Task::Daily,
            model: ModelRef::Local {
                file: "m.gguf".into(),
            },
            reasoning: Some(ReasoningLevel::High),
            ctx_size: None,
            parallel_slots: None,
            batch_size: None,
        }];
        open_model(&ai, Task::Daily, server.port)
            .unwrap()
            .chat("sys", "user")
            .await
            .unwrap();
        let body = &server.received()[0].body;
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
    }

    /// 云端路由时落库的 model 必须是云端模型 ID，不能误取本地文件名。
    #[test]
    fn cloud_route_uses_cloud_model_label() {
        let mut ai = AiConfig::default();
        ai.cloud = vec![crate::ai::config::CloudConnection {
            id: "c1".into(),
            name: "DeepSeek".into(),
            dialect: "deepseek".into(),
            endpoint: "https://api.example.com/v1/".into(),
            model: "deepseek-chat".into(),
            api_key: String::new(),
            context_length: None,
        }];
        ai.picks.daily = Some(ModelRef::Cloud {
            connection: "c1".into(),
        });
        let s = open_model(&ai, Task::Daily, 8080).unwrap();
        assert!(!s.is_local());
        assert_eq!(s.model_label(), "deepseek-chat");
    }

    /// 云端连接未填全或已删除时，必须在构造期直接报 InvalidInput。
    #[test]
    fn cloud_missing_config_errors() {
        let mut ai = AiConfig::default();
        ai.cloud = vec![crate::ai::config::CloudConnection {
            id: "c1".into(),
            name: "bad".into(),
            dialect: "openai".into(),
            endpoint: "   ".into(),
            model: "deepseek-chat".into(),
            api_key: String::new(),
            context_length: None,
        }];
        ai.picks.daily = Some(ModelRef::Cloud {
            connection: "c1".into(),
        });
        assert!(
            matches!(open_model(&ai, Task::Daily, 0), Err(Error::InvalidInput(_))),
            "blank endpoint should return InvalidInput"
        );

        ai.cloud[0].endpoint = "https://api.example.com/v1".into();
        ai.cloud[0].model = "  ".into();
        assert!(
            matches!(open_model(&ai, Task::Daily, 0), Err(Error::InvalidInput(_))),
            "blank model should return InvalidInput"
        );

        ai.picks.daily = Some(ModelRef::Cloud {
            connection: "missing".into(),
        });
        assert!(
            matches!(open_model(&ai, Task::Daily, 0), Err(Error::InvalidInput(_))),
            "dangling connection id should return InvalidInput"
        );
    }
}
