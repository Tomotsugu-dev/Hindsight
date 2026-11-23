//! 按设置打开日报、周报用的模型。

use crate::ai::config::{AiConfig, ModelRef, Task, CLOUD_UNAVAILABLE_ERROR, NO_MODEL_ERROR};
use crate::ai::llm::Step2Chat;
use crate::error::{Error, Result};

/// Opens the model configured for a daily or weekly report with its reasoning level.
///
/// When using a local model, `local_port` is the port of the running llama-server instance.
pub(crate) fn build_step2(ai: &AiConfig, task: Task, local_port: u16) -> Result<Step2Chat> {
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
    fn step2_local_and_missing_routes() {
        let mut ai = AiConfig::default();
        assert!(matches!(
            build_step2(&ai, Task::Daily, 8080),
            Err(Error::InvalidInput(_))
        ));

        ai.picks.daily = Some(ModelRef::Local {
            file: "qwen.gguf".to_string(),
        });
        let s = build_step2(&ai, Task::Daily, 8080).unwrap();
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
        build_step2(&ai, Task::Daily, server.port)
            .unwrap()
            .chat("sys", "user")
            .await
            .unwrap();
        let body = &server.received()[0].body;
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
    }

    /// 云端路由时落库 model 必须是云端连接的模型名——取成本地 GGUF 文件名
    /// 会让 DailyTab / 导出 Markdown 显示错误的生成来源。
    #[test]
    fn step2_cloud_route_uses_cloud_model_label() {
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
        let s = build_step2(&ai, Task::Daily, 8080).unwrap();
        assert!(!s.is_local());
        assert_eq!(s.model_label(), "deepseek-chat");
    }

    /// 选了云端但连接没填全或已删除：必须构造期就报 InvalidInput。顶层错误条按这个类型
    /// 显示「去补配置」的引导；悄悄构造一个打空地址的 client，到运行期才失败，
    /// 用户只会看到一条没法照着做的通用报错。
    #[test]
    fn step2_cloud_missing_config_errors() {
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
            matches!(
                build_step2(&ai, Task::Daily, 0),
                Err(Error::InvalidInput(_))
            ),
            "blank endpoint should return InvalidInput"
        );

        ai.cloud[0].endpoint = "https://api.example.com/v1".into();
        ai.cloud[0].model = "  ".into();
        assert!(
            matches!(
                build_step2(&ai, Task::Daily, 0),
                Err(Error::InvalidInput(_))
            ),
            "blank model should return InvalidInput"
        );

        ai.picks.daily = Some(ModelRef::Cloud {
            connection: "missing".into(),
        });
        assert!(
            matches!(
                build_step2(&ai, Task::Daily, 0),
                Err(Error::InvalidInput(_))
            ),
            "dangling connection id should return InvalidInput"
        );
    }
}
