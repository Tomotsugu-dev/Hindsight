//! 日报和周报调模型的入口。

use crate::ai::model::{LanguageModel, LanguageModelOps, Message, Reasoning, Request};
use crate::ai::providers::chat_completions::ChatCompletionsModel;
use crate::error::Result;

/// 日报和周报用的模型。
#[derive(Clone)]
pub struct Step2Chat {
    model: LanguageModel,
    label: String,
    max_tokens: u32,
    reasoning: Reasoning,
}

impl Step2Chat {
    /// 打开本机 llama-server 上的模型。
    ///
    /// `model` 是 GGUF 文件名。`max_tokens` 是回答的 token 上限，见 [`Request::max_output_tokens`]。
    /// 每次请求都带 `reasoning`。
    pub fn local(port: u16, model: &str, max_tokens: u32, reasoning: Reasoning) -> Result<Self> {
        Ok(Self {
            model: LanguageModel::ChatCompletions(ChatCompletionsModel::local(port, model)?),
            label: model.to_string(),
            max_tokens,
            reasoning,
        })
    }

    /// 打开云端模型。前四个参数同 [`ChatCompletionsModel::cloud`]，后两个同 [`Self::local`]。
    pub fn cloud(
        endpoint: &str,
        model: &str,
        api_key: &str,
        provider: &str,
        max_tokens: u32,
        reasoning: Reasoning,
    ) -> Result<Self> {
        Ok(Self {
            model: LanguageModel::ChatCompletions(ChatCompletionsModel::cloud(
                endpoint, model, api_key, provider,
            )?),
            label: model.to_string(),
            max_tokens,
            reasoning,
        })
    }

    /// 发一次纯文本请求，返回回复的正文。
    pub async fn chat(&self, system: &str, user_text: &str) -> Result<String> {
        let req = Request {
            system: system.to_string(),
            messages: vec![Message::User {
                text: user_text.to_string(),
                images: Vec::new(),
            }],
            tools: Vec::new(),
            reasoning: self.reasoning,
            max_output_tokens: self.max_tokens,
            temperature: Some(0.4),
        };
        Ok(self.model.generate(req).await?.text)
    }

    pub fn is_local(&self) -> bool {
        self.model.capabilities().on_device
    }

    /// 写进 `ai_summaries.model` 的模型名：本机是 GGUF 文件名，云端是模型 ID。
    pub fn model_label(&self) -> &str {
        &self.label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::transport::fake::{FakeServer, Reply};
    use serde_json::json;

    /// 真机端到端：打真实端点，确认流式链路通。
    /// 跑法:
    ///   HINDSIGHT_E2E_BASE_URL=https://api.deepseek.com/v1 \
    ///   HINDSIGHT_E2E_MODEL=deepseek-v4-flash \
    ///   HINDSIGHT_E2E_KEY=sk-... \
    ///   cargo test --lib ai::llm::tests::live_ -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_cloud_streaming_roundtrip() {
        let (Ok(base), Ok(model), Ok(key)) = (
            std::env::var("HINDSIGHT_E2E_BASE_URL"),
            std::env::var("HINDSIGHT_E2E_MODEL"),
            std::env::var("HINDSIGHT_E2E_KEY"),
        ) else {
            eprintln!("跳过:未设 HINDSIGHT_E2E_* 三个环境变量");
            return;
        };
        let c = Step2Chat::cloud(&base, &model, &key, "custom", 512, Reasoning::Default)
            .expect("客户端构造");
        let t = std::time::Instant::now();
        let text = c
            .chat("你是一个简洁的助手。", "用一句话说明什么是 TCP 超时。")
            .await
            .expect("流式请求应成功");
        eprintln!(
            "耗时 {}ms | 内容 {} 字符\n---\n{text}\n---",
            t.elapsed().as_millis(),
            text.chars().count(),
        );
        assert!(!text.trim().is_empty(), "流式应拼出非空内容");
    }

    // ---- 以下从网络这一侧锁住日报发请求的行为 ----

    fn cloud(server: &FakeServer) -> Step2Chat {
        Step2Chat::cloud(
            &server.base_url(),
            "m",
            "sk-test",
            "custom",
            4096,
            Reasoning::Default,
        )
        .unwrap()
    }

    /// 一段完整的流：正文一块，末块只带用量，最后 [DONE]。
    fn sse_ok(text: &str) -> Reply {
        Reply::sse(
            &[
                json!({"choices": [{"delta": {"content": text}}]}),
                json!({"choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 3}}),
            ],
            true,
        )
    }

    /// 云端：请求要流式和用量，带鉴权头；回来的流拼出正文。
    #[tokio::test]
    async fn summary_cloud_reads_sse_reply() {
        let server = FakeServer::start(vec![sse_ok("今天在写测试。")]).await;
        let text = cloud(&server).chat("sys", "user").await.unwrap();
        assert_eq!(text, "今天在写测试。");
        let got = server.received();
        assert_eq!(got[0].body["stream"], true);
        assert_eq!(got[0].body["stream_options"]["include_usage"], true);
        assert!(got[0].head.contains("authorization: bearer sk-test"));
    }

    /// 云端回 400 并写明该换哪个字段名：改名后重发，第二次成功。
    /// 思考档位是 Default，模型可能思考，所以上限是思考底线 12288。
    #[tokio::test]
    async fn summary_cloud_heals_400_then_succeeds() {
        let server = FakeServer::start(vec![
            Reply::json(
                400,
                json!({"error": {"message": "Unsupported parameter: 'max_completion_tokens' is not supported with this model. Use 'max_tokens' instead."}}),
            ),
            sse_ok("好"),
        ])
        .await;
        let text = cloud(&server).chat("sys", "user").await.unwrap();
        assert_eq!(text, "好");
        let got = server.received();
        assert_eq!(got.len(), 2, "只重发一次");
        assert!(got[1].body.get("max_completion_tokens").is_none());
        assert_eq!(got[1].body["max_tokens"], 12_288);
    }

    /// 云端限流：等一会儿原样重发，第二次成功。
    #[tokio::test]
    async fn summary_cloud_backs_off_on_429_then_succeeds() {
        let server = FakeServer::start(vec![
            Reply::json(429, json!({"error": {"message": "rate limited"}}))
                .header("Retry-After", "1"),
            sse_ok("好"),
        ])
        .await;
        let text = cloud(&server).chat("sys", "user").await.unwrap();
        assert_eq!(text, "好");
        let got = server.received();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].body, got[1].body, "限流后原样重发");
    }

    /// 云端的流没收到 [DONE] 就断了：原样重发，第二次成功。
    #[tokio::test]
    async fn summary_cloud_retries_cut_stream_then_succeeds() {
        let server = FakeServer::start(vec![
            Reply::sse(
                &[json!({"choices": [{"delta": {"content": "半截"}}]})],
                false,
            ),
            sse_ok("完整"),
        ])
        .await;
        let text = cloud(&server).chat("sys", "user").await.unwrap();
        assert_eq!(text, "完整", "半截的流不能当结果");
        assert_eq!(server.received().len(), 2);
    }

    /// 本机：温度 0.4，关思考，上限就是给的回答上限。
    #[tokio::test]
    async fn summary_local_fixes_temperature_and_turns_thinking_off() {
        let server = FakeServer::start(vec![Reply::json(
            200,
            json!({"choices": [{"message": {"role": "assistant", "content": "好"}}]}),
        )])
        .await;
        let local = Step2Chat::local(server.port, "m.gguf", 1024, Reasoning::Default).unwrap();
        assert!(local.is_local());
        assert_eq!(local.model_label(), "m.gguf");
        assert_eq!(local.chat("sys", "user").await.unwrap(), "好");
        let body = &server.received()[0].body;
        assert_eq!(body["temperature"], 0.4);
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
    }

    /// 本机 llama-server 回 400：不改请求、不重发，直接报错。
    #[tokio::test]
    async fn summary_local_400_fails_without_retry() {
        let server = FakeServer::start(vec![Reply::json(
            400,
            json!({"error": {"message": "Unknown parameter: 'temperature'."}}),
        )])
        .await;
        let err = Step2Chat::local(server.port, "m.gguf", 1024, Reasoning::Default)
            .unwrap()
            .chat("sys", "user")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("400"), "{err}");
        assert_eq!(server.received().len(), 1);
    }
}
