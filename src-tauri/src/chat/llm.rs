//! Chat 的 LLM 双适配器:云端原生 tools 协议 / 本地 llama-server grammar JSON。
//!
//! 两者输出统一为 [`StepOut`]:要么调一个工具,要么给最终答案——
//! 循环器(engine)对适配器无感知。
//!
//! - 云端:OpenAI 兼容 `tools` + `tool_calls`(厂商训练过的 function calling);
//! - 本地:`json_schema` 参数做 grammar 约束解码——模型在采样层面写不出
//!   非法格式(四道墙的第①道),字段值的语义错误由 tools::validate(第②道)拦。

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};

/// 一步的产出:调工具 or 作答。
/// `id`/`raw` 只有云端有:`raw` 是模型返回的完整 assistant 消息,回放时必须
/// 原样带回——thinking 类模型(如 DeepSeek)要求 `reasoning_content` 一并传回,
/// 自己重构消息会被 400 拒。
/// 单次 LLM 调用的 token 用量(OpenAI 兼容 usage 字段;缺失时为 0)。
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub prompt: u64,
    pub completion: u64,
}

impl TokenUsage {
    fn from_resp(resp: &Value) -> Self {
        Self {
            prompt: resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            completion: resp["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        }
    }
}

#[derive(Debug)]
pub enum StepOut {
    Call {
        name: String,
        args: Value,
        id: Option<String>,
        raw: Option<Value>,
    },
    Final(String),
}

/// 循环器维护的对话条目(线性追加,两种适配器各自渲染成自己的报文格式)。
#[derive(Debug, Clone)]
pub enum Turn {
    User(String),
    AssistantText(String),
    /// 模型发起的工具调用。`raw` = 云端返回的完整 assistant 消息(原样回放);
    /// 本地无 raw,按 name/args 渲染成文本。
    AssistantCall {
        id: String,
        name: String,
        args: String,
        raw: Option<Value>,
    },
    /// 工具执行结果(或参数校验错误——同样走这个通道回填给模型)
    ToolResult {
        id: String,
        content: String,
    },
}

/// 三个工具的 OpenAI function 定义(云端下发;本地版画进 system prompt)。
pub fn tools_schema() -> Value {
    let date =
        |desc: &str| json!({"type": "string", "description": format!("{desc},格式 YYYY-MM-DD")});
    json!([
        {"type": "function", "function": {
            "name": "search_text",
            "description": "全文搜索屏幕上出现过的文字(聊天/网页/代码/订单等,逐字索引)。没命中时换同义词、英文或更短的词重试。",
            "parameters": {"type": "object", "properties": {
                "keywords": {"type": "array", "items": {"type": "string"}, "description": "1-3 个关键词,逐字匹配"},
                "date_from": date("起始日期,可选"),
                "date_to": date("结束日期,可选")
            }, "required": ["keywords"]}
        }},
        {"type": "function", "function": {
            "name": "query_stats",
            "description": "统计应用/内容的使用时长或使用次数。可按应用名过滤(apps)、按窗口标题关键词过滤(title_keyword,如视频名),可分组排行。问'用了多久/花了多少时间'用默认(metric=duration);问'启动/打开/玩了几次'用 metric=session_count。",
            "parameters": {"type": "object", "properties": {
                "date_from": date("起始日期"),
                "date_to": date("结束日期"),
                "apps": {"type": "array", "items": {"type": "string"}, "description": "应用名过滤,可选"},
                "title_keyword": {"type": "string", "description": "窗口标题关键词过滤,可选"},
                "group_by": {"type": "string", "enum": ["none", "app", "title"], "description": "分组维度,默认 none"},
                "top_n": {"type": "integer", "description": "分组时取前 N,默认 5"},
                "metric": {"type": "string", "enum": ["duration", "session_count"], "description": "统计口径:duration=累计时长(默认);session_count=使用会话次数"},
                "gap_minutes": {"type": "integer", "description": "会话计数用:相邻活动间隔超过这么多分钟算一段新会话,默认 30。仅用户明确说'离开X分钟以上算一次'时才填"}
            }, "required": ["date_from", "date_to"]}
        }},
        {"type": "function", "function": {
            "name": "get_timeline",
            "description": "列出某时段的屏幕活动会话(时间、应用、标题),回答'某天/某下午在干什么'。",
            "parameters": {"type": "object", "properties": {
                "date_from": date("起始日期"),
                "date_to": date("结束日期")
            }, "required": ["date_from", "date_to"]}
        }}
    ])
}

/// 本地 grammar 用的"决策对象"schema:扁平单对象,比 oneOf 对小模型稳得多。
/// action=answer 时读 answer 字段,否则按工具读参数字段。
fn local_decision_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": ["search_text", "query_stats", "get_timeline", "answer"]},
            "keywords": {"type": "array", "items": {"type": "string", "maxLength": 64}, "maxItems": 3},
            "date_from": {"type": "string", "maxLength": 10},
            "date_to": {"type": "string", "maxLength": 10},
            "apps": {"type": "array", "items": {"type": "string", "maxLength": 64}, "maxItems": 5},
            "title_keyword": {"type": "string", "maxLength": 64},
            "group_by": {"type": "string", "enum": ["none", "app", "title"]},
            "top_n": {"type": "integer", "minimum": 1, "maximum": 10},
            "metric": {"type": "string", "enum": ["duration", "session_count"]},
            "gap_minutes": {"type": "integer", "minimum": 5, "maximum": 240},
            "answer": {"type": "string"}
        },
        "required": ["action"]
    })
}

/// Chat LLM 客户端:云端 or 本地,一个 step 接口。
pub enum ChatLlm {
    Cloud {
        base_url: String,
        model: String,
        api_key: String,
        http: reqwest::Client,
    },
    Local {
        base_url: String,
        model: String,
        http: reqwest::Client,
    },
}

const CHAT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const MAX_ANSWER_TOKENS: u32 = 1024;

impl ChatLlm {
    pub fn cloud(endpoint: &str, model: String, api_key: String) -> Result<Self> {
        let base_url = endpoint.trim().trim_end_matches('/').to_string();
        if base_url.is_empty() || model.trim().is_empty() {
            return Err(Error::InvalidInput("云端 API 地址或模型 ID 为空"));
        }
        Ok(Self::Cloud {
            base_url,
            model,
            api_key,
            http: http_client()?,
        })
    }

    pub fn local(port: u16, model: String) -> Result<Self> {
        Ok(Self::Local {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            model,
            http: http_client()?,
        })
    }

    /// 跑一步:给定 system + 对话,产出"调工具"或"作答"。
    pub async fn step(&self, system: &str, turns: &[Turn]) -> Result<(StepOut, TokenUsage)> {
        match self {
            Self::Cloud { .. } => self.step_cloud(system, turns).await,
            Self::Local { .. } => self.step_local(system, turns).await,
        }
    }

    async fn step_cloud(&self, system: &str, turns: &[Turn]) -> Result<(StepOut, TokenUsage)> {
        let Self::Cloud {
            base_url,
            model,
            api_key,
            http,
        } = self
        else {
            unreachable!()
        };
        let mut messages = vec![json!({"role": "system", "content": system})];
        for t in turns {
            messages.push(match t {
                Turn::User(c) => json!({"role": "user", "content": c}),
                Turn::AssistantText(c) => json!({"role": "assistant", "content": c}),
                Turn::AssistantCall { raw: Some(raw), .. } => raw.clone(),
                Turn::AssistantCall {
                    id,
                    name,
                    args,
                    raw: None,
                } => json!({
                    "role": "assistant",
                    "tool_calls": [{"id": id, "type": "function",
                        "function": {"name": name, "arguments": args}}]
                }),
                Turn::ToolResult { id, content } => {
                    json!({"role": "tool", "tool_call_id": id, "content": content})
                }
            });
        }
        let body = json!({
            "model": model,
            "messages": messages,
            "tools": tools_schema(),
            "tool_choice": "auto",
            "max_tokens": MAX_ANSWER_TOKENS,
        });
        let mut req = http
            .post(format!("{base_url}/chat/completions"))
            .json(&body);
        if !api_key.trim().is_empty() {
            req = req.bearer_auth(api_key.trim());
        }
        let resp: Value = send_json(req).await?;
        let usage = TokenUsage::from_resp(&resp);
        let msg = &resp["choices"][0]["message"];
        if let Some(call) = msg["tool_calls"].get(0) {
            let name = call["function"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");
            let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            let id = call["id"].as_str().map(str::to_string);
            // 原样保留 assistant 消息用于回放;一次多个 tool_calls 时只留第一个
            // (我们只执行第一个,回放多余的 id 会因缺对应 tool 结果被 API 拒)
            let mut raw = msg.clone();
            if let Some(calls) = raw["tool_calls"].as_array_mut() {
                calls.truncate(1);
            }
            return Ok((
                StepOut::Call {
                    name,
                    args,
                    id,
                    raw: Some(raw),
                },
                usage,
            ));
        }
        let content = msg["content"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string();
        if content.is_empty() {
            return Err(Error::LlmResponse("模型返回空内容".into()));
        }
        Ok((StepOut::Final(content), usage))
    }

    async fn step_local(&self, system: &str, turns: &[Turn]) -> Result<(StepOut, TokenUsage)> {
        let Self::Local {
            base_url,
            model,
            http,
        } = self
        else {
            unreachable!()
        };
        // 本地:工具协议画在文本里,输出被 json_schema(grammar)约束成决策对象
        let mut transcript = String::new();
        for t in turns {
            match t {
                Turn::User(c) => transcript.push_str(&format!("用户: {c}\n")),
                Turn::AssistantText(c) => transcript.push_str(&format!("助手: {c}\n")),
                Turn::AssistantCall { name, args, .. } => {
                    transcript.push_str(&format!("助手(调用工具): {name} {args}\n"))
                }
                Turn::ToolResult { content, .. } => {
                    transcript.push_str(&format!("工具结果:\n{content}\n"))
                }
            }
        }
        transcript.push_str("请输出下一步决策(JSON):");
        let body = json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": transcript},
            ],
            "max_tokens": MAX_ANSWER_TOKENS,
            // llama-server 扩展参数:按 JSON schema 生成 grammar,采样层强约束
            "json_schema": local_decision_schema(),
        });
        let resp: Value = send_json(
            http.post(format!("{base_url}/chat/completions"))
                .json(&body),
        )
        .await?;
        let usage = TokenUsage::from_resp(&resp);
        let content = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default();

        #[derive(Deserialize)]
        struct Decision {
            action: String,
            answer: Option<String>,
            #[serde(flatten)]
            rest: Value,
        }
        let d: Decision = serde_json::from_str(content)
            .map_err(|e| Error::LlmResponse(format!("决策 JSON 解析失败: {e}")))?;
        if d.action == "answer" {
            let text = d.answer.unwrap_or_default();
            if text.trim().is_empty() {
                return Err(Error::LlmResponse("answer 为空".into()));
            }
            return Ok((StepOut::Final(text), usage));
        }
        Ok((
            StepOut::Call {
                name: d.action,
                args: d.rest,
                id: None,
                raw: None,
            },
            usage,
        ))
    }
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(CHAT_TIMEOUT)
        .build()
        .map_err(|e| Error::LlmResponse(format!("HTTP 客户端构造失败: {e}")))
}

async fn send_json(req: reqwest::RequestBuilder) -> Result<Value> {
    let resp = req
        .send()
        .await
        .map_err(|e| Error::LlmResponse(format!("请求失败: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| Error::LlmResponse(format!("读响应失败: {e}")))?;
    if !status.is_success() {
        return Err(Error::LlmResponse(format!(
            "HTTP {status}: {}",
            &text[..text.len().min(300)]
        )));
    }
    serde_json::from_str(&text).map_err(|e| Error::LlmResponse(format!("响应不是 JSON: {e}")))
}
