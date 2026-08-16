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
    /// 思考(reasoning)消耗,已含在 `completion` 里。
    /// `completion_tokens_details.reasoning_tokens` 是三家通用字段(实测
    /// DeepSeek / OpenAI / Moonshot 都给),关闭思考时对象整个不返回 = 0。
    /// 本地 llama-server 不给此字段,恒 0。
    pub reasoning: u64,
}

impl TokenUsage {
    fn from_resp(resp: &Value) -> Self {
        Self {
            prompt: resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            completion: resp["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            reasoning: resp["usage"]["completion_tokens_details"]["reasoning_tokens"]
                .as_u64()
                .unwrap_or(0),
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
            "description": "全文搜索屏幕上出现过的文字(聊天/网页/代码/订单等,逐字索引)。返回头部标注总命中数,正文为相关度最高的若干条。没命中时换同义词、英文或更短的词重试。",
            "parameters": {"type": "object", "properties": {
                "keywords": {"type": "array", "items": {"type": "string"}, "description": "1-3 个关键词,逐字匹配"},
                "date_from": date("起始日期,可选"),
                "date_to": date("结束日期,可选")
            }, "required": ["keywords"]}
        }},
        {"type": "function", "function": {
            "name": "query_stats",
            "description": "统计应用/内容的使用时长或使用次数。可按应用名过滤(apps)、按分类名过滤(categories)、按窗口标题关键词过滤(title_keyword,如视频名),可分组排行(group_by,含按用户分类的工作/娱乐口径),可分桶看趋势(bucket)。问'用了多久/花了多少时间'用默认(metric=duration);问'启动/打开/玩了几次'用 metric=session_count;问'第一次/最近一次用X是什么时候'用 metric=first_last(此口径 date_from 可给很早的日期,如 2020-01-01)。问某类活动(如游戏)的趋势用 categories+bucket,不要自己列举应用名——用户归类过的应用你未必认识。",
            "parameters": {"type": "object", "properties": {
                "date_from": date("起始日期"),
                "date_to": date("结束日期"),
                "apps": {"type": "array", "items": {"type": "string"}, "description": "应用名过滤,可选"},
                "categories": {"type": "array", "items": {"type": "string"}, "description": "分类名过滤(如'游戏'),按用户的应用分类圈定范围,可选;分类名可先用 group_by=category 查到"},
                "title_keyword": {"type": "string", "description": "窗口标题关键词过滤,可选"},
                "group_by": {"type": "string", "enum": ["none", "app", "title", "category"], "description": "分组维度,默认 none;category=按用户设置的应用分类(工作/娱乐/游戏等)分组"},
                "top_n": {"type": "integer", "description": "分组时取前 N,默认 5"},
                "metric": {"type": "string", "enum": ["duration", "session_count", "first_last"], "description": "统计口径:duration=累计时长(默认);session_count=使用会话次数;first_last=最早与最近一次记录的时间(忽略分组/分桶)"},
                "gap_minutes": {"type": "integer", "description": "会话计数用:相邻活动间隔超过这么多分钟算一段新会话,默认 30。仅用户明确说'离开X分钟以上算一次'时才填"},
                "bucket": {"type": "string", "enum": ["none", "day", "week", "hour_of_day"], "description": "趋势分桶,与 group_by 互斥:day=逐日(最多 60 天,更长自动按周);week=逐周(行首日期为周一);hour_of_day=按一天 0-23 时聚合看作息分布"}
            }, "required": ["date_from", "date_to"]}
        }},
        {"type": "function", "function": {
            "name": "get_timeline",
            "description": "按小时抽样列出某时段的屏幕活动会话(时间、应用、标题),回答'某天/某下午在干什么'。返回头部标注总条数与覆盖范围,正文是抽样代表,不是全量。",
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
            "categories": {"type": "array", "items": {"type": "string", "maxLength": 64}, "maxItems": 5},
            "title_keyword": {"type": "string", "maxLength": 64},
            "group_by": {"type": "string", "enum": ["none", "app", "title", "category"]},
            "top_n": {"type": "integer", "minimum": 1, "maximum": 10},
            "metric": {"type": "string", "enum": ["duration", "session_count", "first_last"]},
            "gap_minutes": {"type": "integer", "minimum": 5, "maximum": 240},
            "bucket": {"type": "string", "enum": ["none", "day", "week", "hour_of_day"]},
            "answer": {"type": "string"}
        },
        "required": ["action"]
    })
}

/// Chat 思考模式偏好(设置 `ai.chat_thinking` 的解析形态)。
/// 强度档的值域各家不同(实测+文档,2026-08):DeepSeek low/high/max、
/// OpenAI(gpt-5.1+)none/low/medium/high、OpenRouter low/medium/high、
/// 本地与其它云端只有开关。UI 按当前服务商只展示它真实存在的档位,
/// 注入层对"切换服务商后残留的值域外档"就近降级,绝不发值域外字面量
/// (DeepSeek 实测对值域外 effort 200 静默,但不赌其它家)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingMode {
    Auto,
    Off,
    Low,
    Medium,
    High,
    Max,
}

impl ThinkingMode {
    pub fn from_setting(s: &str) -> Self {
        match s {
            // "on" 是分档前的旧值(设置可能还没被 sanitize 重写):当高档
            "on" | "high" => Self::High,
            "max" => Self::Max,
            "medium" => Self::Medium,
            "low" => Self::Low,
            "off" => Self::Off,
            _ => Self::Auto,
        }
    }

    /// low/medium/high 值域(OpenAI/OpenRouter)的 effort 字面量;
    /// Max 就近降为 high,Auto/Off 为 None。
    fn effort_lmh(self) -> Option<&'static str> {
        match self {
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High | Self::Max => Some("high"),
            Self::Auto | Self::Off => None,
        }
    }

    /// 是否任一强度档(= 要求开思考)。
    fn wants_thinking(self) -> bool {
        !matches!(self, Self::Auto | Self::Off)
    }
}

/// 云端请求体的思考控制注入(字段逐家查证 + `scripts/llm/thinking_probe.py`
/// 实测,2026-08):
/// - `Auto` 一个字节都不加——实测 DeepSeek 对未知字段 200 静默忽略,但
///   OpenAI 对未知参数会 400,零注入是唯一零风险基线;
/// - deepseek:`thinking.type` enabled/disabled(官方字段;实测 disabled 把
///   completion 从 ~100 降到个位数 token,延迟同降)——无强度分档,
///   高/中/低都发 enabled;
/// - openrouter:强度档发 `reasoning.effort`(隐含 enabled),关闭发
///   `reasoning.enabled=false`(不能与顶层 reasoning_effort 混发,否则 400);
/// - openai:`reasoning_effort` high/medium/low/none;
/// - 其它(together/groq/custom):`chat_template_kwargs.enable_thinking`
///   (vLLM/SGLang 生态惯例,不认的端点以忽略为主),同样只有开关。
fn inject_cloud_thinking(body: &mut Value, provider: &str, mode: ThinkingMode, has_tools: bool) {
    // OpenAI 官方的硬限制(实测 gpt-5.6-luna,2026-08):Chat Completions 里
    // function tools 与 reasoning_effort≠none **不能共存**,连"不发参数"都
    // 会撞上(模型默认档非 none)——`Function tools with reasoning_effort are
    // not supported ... use the Responses API`。Chat 主路径永远带 tools,
    // 所以这里必须显式发 none,否则 OpenAI 一句话都答不了。
    // 不带 tools 的调用(问题自立化改写器)不受此限,按用户档位正常下发。
    if provider == "openai" && has_tools {
        body["reasoning_effort"] = json!("none");
        return;
    }
    if mode == ThinkingMode::Auto {
        return;
    }
    let on = mode.wants_thinking();
    match provider {
        // 开关走 thinking.type;强度走独立的 reasoning_effort(实测两参数独立,
        // disabled 时 effort 被忽略)。官方值域 low/high/max,无 medium——
        // UI 在 deepseek 下不出"中"档,这里的 Medium 只是切换服务商的残留,
        // 就近降为 high(=官方默认强度)。
        "deepseek" => {
            body["thinking"] = json!({"type": if on { "enabled" } else { "disabled" }});
            let ds = match mode {
                ThinkingMode::Low => Some("low"),
                ThinkingMode::Medium | ThinkingMode::High => Some("high"),
                ThinkingMode::Max => Some("max"),
                ThinkingMode::Auto | ThinkingMode::Off => None,
            };
            if let Some(e) = ds {
                body["reasoning_effort"] = json!(e);
            }
        }
        // effort 隐含 enabled;关闭时只发 enabled=false(effort 与其互斥)。
        // 值域 low/medium/high:Max 残留就近 high
        "openrouter" => {
            body["reasoning"] = match mode.effort_lmh() {
                Some(e) => json!({"effort": e}),
                None => json!({"enabled": false}),
            };
        }
        // openai:gpt-5.1+ 值域 none/low/medium/high(none 为关),Max 残留就近 high。
        // kimi(Moonshot):同款 reasoning_effort 字段,实测 kimi-k3 各档思考量
        // 单调递增(none 0 字 / low 133 / medium 275 / high 372),且与 tools
        // 共存无碍——OpenAI 那条 tools 限制不适用于它。
        // 注意:个别模型(kimi-k2.7-code)强制思考,发 none 会 400。
        "openai" | "kimi" | "kimi-cn" => {
            body["reasoning_effort"] = json!(mode.effort_lmh().unwrap_or("none"));
        }
        _ => {
            body["chat_template_kwargs"] = json!({"enable_thinking": on});
        }
    }
}

/// 本地 llama-server 的思考注入。与云端不同,`Auto` 也注入 false:实测
/// (Qwen3.5-4B)hybrid 模型默认思考,在 grammar(json_schema)约束 + 有限
/// max_tokens 下思考吃光预算、决策 JSON 根本没机会输出——"默认不管"在
/// 本地等于默认坏。强度档时调用方需同时放大 max_tokens(见
/// [`LOCAL_THINKING_MAX_TOKENS`]),否则必然空输出。
fn inject_local_thinking(body: &mut Value, mode: ThinkingMode) {
    // 本地无强度可言:任一强度档=开(llama-server 只认布尔开关,
    // 预算也不按档分——4096 实测已被完整思考链吃满,低档只会更早窒息)
    body["chat_template_kwargs"] = json!({"enable_thinking": mode.wants_thinking()});
}

/// 本地开思考时的 max_tokens:完整思考链实测 1-2K token,再留决策 JSON 的份。
const LOCAL_THINKING_MAX_TOKENS: u32 = 4096;

/// 云端非关闭态的 max_tokens:思考 token 计入 completion,实测 DeepSeek
/// high 档思考 ~5K token、max 档 6K+,1024 会被思考打满、正文截断为空。
/// Auto 也要用它——DeepSeek 默认思考开,Auto 不注入参数照样思考。
const CLOUD_THINKING_MAX_TOKENS: u32 = 12288;

/// Chat LLM 客户端:云端 or 本地,一个 step 接口。
pub enum ChatLlm {
    Cloud {
        base_url: String,
        model: String,
        api_key: String,
        /// 服务商预设(ai.external_provider),思考注入按它分发
        provider: String,
        thinking: ThinkingMode,
        http: reqwest::Client,
    },
    Local {
        base_url: String,
        model: String,
        thinking: ThinkingMode,
        http: reqwest::Client,
    },
}

// 请求自愈与输出预算字段名与 ai::llm(日报/周报)共用同一份实现——
// 两套 OpenAI 兼容传输层曾各自演化出分歧:聊天发 max_completion_tokens
// 且带自愈,摘要硬编码 max_tokens 且无自愈,导致同一端点下聊天可用、
// 日报永久 400。共用后新规则一次生效两边。
use crate::ai::openai_compat::{budget_key, heal_request, MAX_HEAL_ROUNDS};

const CHAT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const MAX_ANSWER_TOKENS: u32 = 1024;

impl ChatLlm {
    pub fn cloud(
        endpoint: &str,
        model: String,
        api_key: String,
        provider: String,
        thinking: ThinkingMode,
    ) -> Result<Self> {
        let base_url = endpoint.trim().trim_end_matches('/').to_string();
        if base_url.is_empty() || model.trim().is_empty() {
            return Err(Error::InvalidInput("云端 API 地址或模型 ID 为空"));
        }
        Ok(Self::Cloud {
            base_url,
            model,
            api_key,
            provider,
            thinking,
            http: http_client()?,
        })
    }

    pub fn local(port: u16, model: String, thinking: ThinkingMode) -> Result<Self> {
        Ok(Self::Local {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            model,
            thinking,
            http: http_client()?,
        })
    }

    /// 纯文本一问一答(无工具、无 grammar 约束):多轮"问题自立化"改写器用。
    /// 两端都走 OpenAI 兼容 /chat/completions,只发 system + user 两条消息。
    pub async fn complete(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<(String, TokenUsage)> {
        let (base_url, model, api_key, http) = match self {
            Self::Cloud {
                base_url,
                model,
                api_key,
                http,
                ..
            } => (base_url, model, api_key.trim(), http),
            Self::Local {
                base_url,
                model,
                http,
                ..
            } => (base_url, model, "", http),
        };
        let mut body = json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "temperature": 0,
        });
        match self {
            Self::Cloud {
                provider, thinking, ..
            } => {
                // 改写器不带 tools,思考档位可原样下发(带 tools 的限制见 step_cloud)
                inject_cloud_thinking(&mut body, provider, *thinking, false);
                // 非关闭态给足思考预算(Auto 下 DeepSeek 也默认思考)
                let budget = if matches!(*thinking, ThinkingMode::Off) {
                    max_tokens
                } else {
                    max_tokens.max(CLOUD_THINKING_MAX_TOKENS)
                };
                body[budget_key(true)] = json!(budget);
            }
            Self::Local { thinking, .. } => {
                inject_local_thinking(&mut body, *thinking);
                let budget = if thinking.wants_thinking() {
                    max_tokens.max(LOCAL_THINKING_MAX_TOKENS)
                } else {
                    max_tokens
                };
                body[budget_key(false)] = json!(budget);
            }
        }
        let url = format!("{base_url}/chat/completions");
        let resp: Value = send_healing(http, &url, api_key, &mut body).await?;
        let usage = TokenUsage::from_resp(&resp);
        let content = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string();
        if content.is_empty() {
            return Err(Error::LlmResponse("模型返回空内容".into()));
        }
        Ok((content, usage))
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
            provider,
            thinking,
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
        // 非关闭态思考 token 挤占 completion 预算(Auto 下 DeepSeek 默认思考,
        // 实测 1024 会被复杂问题的思考链打满、正文截空)
        let max_tokens = if matches!(thinking, ThinkingMode::Off) {
            MAX_ANSWER_TOKENS
        } else {
            CLOUD_THINKING_MAX_TOKENS
        };
        let mut body = json!({
            "model": model,
            "messages": messages,
            "tools": tools_schema(),
            "tool_choice": "auto",
        });
        body[budget_key(true)] = json!(max_tokens);
        inject_cloud_thinking(&mut body, provider, *thinking, true);
        let url = format!("{base_url}/chat/completions");
        let resp: Value = send_healing(http, &url, api_key.trim(), &mut body).await?;
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
            thinking,
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
        // 开思考时预算放大:思考链先于 grammar 决策输出,预算不够就只剩思考
        let max_tokens = if thinking.wants_thinking() {
            LOCAL_THINKING_MAX_TOKENS
        } else {
            MAX_ANSWER_TOKENS
        };
        let mut body = json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": transcript},
            ],
            "max_tokens": max_tokens,
            // llama-server 扩展参数:按 JSON schema 生成 grammar,采样层强约束
            "json_schema": local_decision_schema(),
        });
        inject_local_thinking(&mut body, *thinking);
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

/// 发请求;400 且能按错误信息修正时自愈重试(见 [`heal_request`])。
/// 非 400(鉴权/限流/网关)一律不重试——那不是参数问题,重发只是浪费。
async fn send_healing(
    http: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &mut Value,
) -> Result<Value> {
    let mut round = 0u32;
    loop {
        let mut req = http.post(url).json(&*body);
        if !api_key.is_empty() {
            req = req.bearer_auth(api_key);
        }
        match send_json(req).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let Error::LlmResponse(msg) = &e else {
                    return Err(e);
                };
                if round >= MAX_HEAL_ROUNDS
                    || !msg.starts_with("HTTP 400")
                    || !heal_request(body, msg)
                {
                    return Err(e);
                }
                log::warn!("chat 云端 400,按错误信息自愈后重试(第 {} 轮)", round + 1);
                round += 1;
            }
        }
    }
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
        // 预览按字符截断:错误体可能是中文/多字节(网关中文错误页),
        // 按字节切 300 会切在字符中间直接 panic(B 档测试实锤的隐患)
        let preview: String = text.chars().take(300).collect();
        return Err(Error::LlmResponse(format!("HTTP {status}: {preview}")));
    }
    serde_json::from_str(&text).map_err(|e| Error::LlmResponse(format!("响应不是 JSON: {e}")))
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    /// 云端 tools schema 与本地 grammar schema 的枚举必须同步扩展——
    /// 新维度只加了一边的话,另一条通路会静默退化(模型想用但被 schema 拒)。
    #[test]
    fn cloud_and_local_schemas_stay_in_sync() {
        let tools = tools_schema();
        let stats = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["function"]["name"] == "query_stats")
            .expect("query_stats 必须存在");
        let props = &stats["function"]["parameters"]["properties"];
        let enum_of = |v: &serde_json::Value| -> Vec<String> {
            v["enum"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect()
        };
        let cloud_group = enum_of(&props["group_by"]);
        let cloud_metric = enum_of(&props["metric"]);
        let cloud_bucket = enum_of(&props["bucket"]);
        assert!(cloud_group.contains(&"category".to_string()));
        assert!(cloud_metric.contains(&"first_last".to_string()));
        assert!(cloud_bucket.contains(&"hour_of_day".to_string()));

        let local = local_decision_schema();
        let lp = &local["properties"];
        assert_eq!(enum_of(&lp["group_by"]), cloud_group);
        assert_eq!(enum_of(&lp["metric"]), cloud_metric);
        assert_eq!(enum_of(&lp["bucket"]), cloud_bucket);
        // categories 两边都有,且本地限制条数
        assert!(props["categories"].is_object());
        assert_eq!(lp["categories"]["maxItems"], 5);
    }

    #[test]
    fn tools_schema_lists_three_tools_with_required_ranges() {
        let tools = tools_schema();
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["search_text", "query_stats", "get_timeline"]);
        for t in tools.as_array().unwrap() {
            let req = t["function"]["parameters"]["required"].as_array().unwrap();
            assert!(
                req.iter().any(|r| r == "date_from") || t["function"]["name"] == "search_text",
                "{} 缺日期必填",
                t["function"]["name"]
            );
        }
    }
}

/// 云端 HTTP 通路的行为测试:用 127.0.0.1 上的一次性假服务喂 canned 响应,
/// 覆盖 step_cloud 的解析各形态、非 2xx 透传、畸形响应容错、拒连/挂死路径,
/// 以及 ai::llm::ExternalChatClient 的空回复归因错误码(云端文本通路的另一半,
/// 其 [LLM_EMPTY_*] 码是前端本地化提示的契约,产生条件必须钉死)。
#[cfg(test)]
mod cloud_http_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// 拿一个刚释放的本地端口——连接必然被拒,模拟"服务没起来/地址配错"。
    fn free_local_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    /// 起一个一次性 HTTP 假服务:读完整个请求(headers + Content-Length body)
    /// 再按给定状态行/响应体回包,并把收到的原始请求全文送回测试侧——
    /// 这样既能断言"响应怎么被解析",也能断言"请求到底发了什么"(鉴权头/报文形状)。
    /// `status_line` 形如 "200 OK" / "401 Unauthorized"。
    async fn spawn_http_once(
        status_line: &str,
        body: String,
    ) -> (u16, tokio::sync::oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let status_line = status_line.to_string();
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
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_line,
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            let _ = sock.shutdown().await;
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
        });
        (port, rx)
    }

    fn cloud_client(port: u16) -> ChatLlm {
        ChatLlm::cloud(
            // 带尾斜杠:顺带验证构造期会 trim,不会拼出 //chat/completions
            &format!("http://127.0.0.1:{port}/v1/"),
            "test-model".into(),
            "sk-test".into(),
            "custom".into(),
            ThinkingMode::Auto,
        )
        .unwrap()
    }

    /// 思考注入矩阵(字段口径见 inject_cloud_thinking 注释,均经真机探针实测):
    /// Auto 必须零注入——不认识字段的服务商(OpenAI)会 400。
    #[test]
    fn thinking_injection_matrix() {
        let base = || json!({"model": "m", "messages": []});
        // Auto:云端一个字节都不加
        for p in ["deepseek", "openrouter", "openai", "custom", "together"] {
            let mut b = base();
            inject_cloud_thinking(&mut b, p, ThinkingMode::Auto, false);
            assert_eq!(b, base(), "Auto 不得注入任何字段(provider={p})");
        }
        // deepseek:thinking.type 开关 + reasoning_effort 强度(官方值域
        // low/high/max,无 medium——Medium 残留就近 high;实弹验证三档真分档:
        // 同题思考 2889/6626/9130 字)
        for (m, want) in [
            (ThinkingMode::Low, "low"),
            (ThinkingMode::Medium, "high"),
            (ThinkingMode::High, "high"),
            (ThinkingMode::Max, "max"),
        ] {
            let mut b = base();
            inject_cloud_thinking(&mut b, "deepseek", m, false);
            assert_eq!(b["thinking"]["type"], "enabled");
            assert_eq!(b["reasoning_effort"], want);
        }
        let mut b = base();
        inject_cloud_thinking(&mut b, "deepseek", ThinkingMode::Off, false);
        assert_eq!(b["thinking"]["type"], "disabled");
        assert!(b.get("reasoning_effort").is_none(), "关闭不发强度");
        // openrouter:强度→reasoning.effort(值域 low/medium/high,Max 残留
        // 就近 high);关→reasoning.enabled=false;都不得出现顶层 reasoning_effort
        for (m, want) in [
            (ThinkingMode::Low, "low"),
            (ThinkingMode::Medium, "medium"),
            (ThinkingMode::High, "high"),
            (ThinkingMode::Max, "high"),
        ] {
            let mut b = base();
            inject_cloud_thinking(&mut b, "openrouter", m, false);
            assert_eq!(b["reasoning"]["effort"], want);
            assert!(b.get("reasoning_effort").is_none());
        }
        let mut b = base();
        inject_cloud_thinking(&mut b, "openrouter", ThinkingMode::Off, false);
        assert_eq!(b["reasoning"]["enabled"], false);
        assert!(b.get("reasoning_effort").is_none());
        // openai:reasoning_effort 逐档直传(gpt-5.1+ 值域 none/low/medium/high,
        // Max 残留就近 high),关=none
        for (m, want) in [
            (ThinkingMode::Low, "low"),
            (ThinkingMode::Medium, "medium"),
            (ThinkingMode::High, "high"),
            (ThinkingMode::Max, "high"),
            (ThinkingMode::Off, "none"),
        ] {
            let mut b = base();
            inject_cloud_thinking(&mut b, "openai", m, false);
            assert_eq!(b["reasoning_effort"], want);
        }
        // 其它/自建:vLLM 生态惯例,只有开关
        let mut b = base();
        inject_cloud_thinking(&mut b, "custom", ThinkingMode::Off, false);
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        let mut b = base();
        inject_cloud_thinking(&mut b, "custom", ThinkingMode::Medium, false);
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], true);

        // 本地:Auto/Off 都必须显式关(hybrid 模型默认思考会吃光 grammar
        // 决策的预算——实测正文为空),任一强度档=开
        for (mode, want) in [
            (ThinkingMode::Auto, false),
            (ThinkingMode::Off, false),
            (ThinkingMode::Low, true),
            (ThinkingMode::Medium, true),
            (ThinkingMode::High, true),
            (ThinkingMode::Max, true),
        ] {
            let mut b = base();
            inject_local_thinking(&mut b, mode);
            assert_eq!(b["chat_template_kwargs"]["enable_thinking"], want);
        }

        // 设置值解析:分档前的旧值 "on" 必须继续当"开"用;非法值回 Auto
        assert_eq!(ThinkingMode::from_setting("on"), ThinkingMode::High);
        assert_eq!(ThinkingMode::from_setting("max"), ThinkingMode::Max);
        assert_eq!(ThinkingMode::from_setting("怪值"), ThinkingMode::Auto);
    }

    /// OpenAI 的硬限制(实测 gpt-5.6-luna):Chat Completions 里 function tools
    /// 与 reasoning_effort≠none 不能共存,连"不发参数"都会被拒(默认档非 none)。
    /// Chat 主路径永远带 tools,所以带 tools 时必须显式发 none——否则一句话都答不了。
    #[test]
    fn openai_with_tools_always_disables_reasoning() {
        let base = || json!({"model": "m", "messages": []});
        for m in [
            ThinkingMode::Auto,
            ThinkingMode::Off,
            ThinkingMode::Low,
            ThinkingMode::High,
            ThinkingMode::Max,
        ] {
            let mut b = base();
            inject_cloud_thinking(&mut b, "openai", m, true);
            assert_eq!(b["reasoning_effort"], "none", "openai+tools 只能发 none");
        }
        // 不带 tools(问题自立化改写器)不受限,档位原样下发
        let mut b = base();
        inject_cloud_thinking(&mut b, "openai", ThinkingMode::High, false);
        assert_eq!(b["reasoning_effort"], "high");
        // 同为 reasoning_effort 家族的 kimi 不受这条限制:带 tools 也发真实档位
        // (实测 Moonshot tools + effort=low/medium/high 全部 200)
        let mut b = base();
        inject_cloud_thinking(&mut b, "kimi", ThinkingMode::Medium, true);
        assert_eq!(b["reasoning_effort"], "medium");
        let mut b = base();
        inject_cloud_thinking(&mut b, "kimi-cn", ThinkingMode::Off, true);
        assert_eq!(b["reasoning_effort"], "none");
    }

    /// HTTP 级:deepseek + Off 的 step 请求体真的带上了 thinking.type=disabled。
    #[tokio::test]
    async fn step_cloud_sends_thinking_field_for_deepseek() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": "好的"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        })
        .to_string();
        let (port, rx) = spawn_http_once("200 OK", body).await;
        let llm = ChatLlm::cloud(
            &format!("http://127.0.0.1:{port}/v1"),
            "test-model".into(),
            String::new(),
            "deepseek".into(),
            ThinkingMode::Off,
        )
        .unwrap();
        llm.step("s", &[Turn::User("问".into())]).await.unwrap();
        let raw = rx.await.unwrap();
        assert!(
            raw.contains(r#""thinking":{"type":"disabled"}"#),
            "请求体应含 deepseek 官方关闭字段,实际:{raw}"
        );
    }

    /// 纯文本作答形态:content 取 choices[0] 并 trim、usage 透传;
    /// tool_calls 为空数组时不能误判成工具调用(OpenAI 部分兼容端会返 [])。
    #[tokio::test]
    async fn step_cloud_final_content_trims_and_reads_usage() {
        let body = json!({
            "choices": [{
                "message": {"role": "assistant", "content": "  今天主要在写测试。 ", "tool_calls": []},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 42, "completion_tokens": 17}
        })
        .to_string();
        let (port, _rx) = spawn_http_once("200 OK", body).await;
        let (out, usage) = cloud_client(port)
            .step("系统提示", &[Turn::User("我今天干了啥".into())])
            .await
            .unwrap();
        match out {
            StepOut::Final(text) => {
                assert_eq!(text, "今天主要在写测试。", "content 应 trim 后原样返回")
            }
            other => panic!("空 tool_calls 数组应走作答分支,实际: {other:?}"),
        }
        assert_eq!(
            (usage.prompt, usage.completion),
            (42, 17),
            "usage 应逐字段透传"
        );
    }

    /// 工具调用形态:name/args/id 逐项解析;一次多个 tool_calls 只执行第一个,
    /// 回放用的 raw 必须把多余的 call 截掉——否则回放时孤儿 id 缺 tool 结果被 API 拒。
    #[tokio::test]
    async fn step_cloud_tool_call_parses_first_and_truncates_raw() {
        let body = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {
                            "name": "query_stats",
                            "arguments": "{\"date_from\":\"2026-07-01\",\"date_to\":\"2026-07-25\"}"
                        }},
                        {"id": "call_2", "type": "function", "function": {
                            "name": "get_timeline", "arguments": "{}"
                        }}
                    ]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 100, "completion_tokens": 30}
        })
        .to_string();
        let (port, _rx) = spawn_http_once("200 OK", body).await;
        let (out, usage) = cloud_client(port)
            .step("s", &[Turn::User("统计一下".into())])
            .await
            .unwrap();
        let StepOut::Call {
            name,
            args,
            id,
            raw,
        } = out
        else {
            panic!("有 tool_calls 应走调用分支");
        };
        assert_eq!(name, "query_stats");
        assert_eq!(args["date_from"], "2026-07-01");
        assert_eq!(args["date_to"], "2026-07-25");
        assert_eq!(id.as_deref(), Some("call_1"), "id 应取第一个 call 的");
        let raw = raw.expect("云端必须带 raw 供回放");
        assert_eq!(
            raw["tool_calls"].as_array().unwrap().len(),
            1,
            "raw 只应保留被执行的第一个 call"
        );
        assert_eq!(raw["tool_calls"][0]["id"], "call_1");
        assert_eq!((usage.prompt, usage.completion), (100, 30));
    }

    /// arguments 不是合法 JSON 时应退化为空对象继续走调用分支(交给下游参数
    /// 校验报错回填),而不是整个 step 失败——模型偶发写坏参数不该断掉整轮对话。
    /// usage 字段整个缺失时按 0 计,不报错。
    #[tokio::test]
    async fn step_cloud_malformed_arguments_fall_back_to_empty_object() {
        let body = json!({
            "choices": [{
                "message": {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {
                        "name": "search_text", "arguments": "{keywords: 不是JSON"
                    }}
                ]},
                "finish_reason": "tool_calls"
            }]
        })
        .to_string();
        let (port, _rx) = spawn_http_once("200 OK", body).await;
        let (out, usage) = cloud_client(port)
            .step("s", &[Turn::User("搜".into())])
            .await
            .unwrap();
        let StepOut::Call { name, args, .. } = out else {
            panic!("坏 arguments 仍应走调用分支");
        };
        assert_eq!(name, "search_text");
        assert_eq!(args, json!({}), "解析失败的 arguments 应退化为 {{}}");
        assert_eq!((usage.prompt, usage.completion), (0, 0), "缺 usage 按 0 计");
    }

    /// 请求侧契约:system 在首位、四种 Turn 各自渲染成正确的 OpenAI 报文形状、
    /// 带 raw 的 AssistantCall 原样回放(thinking 模型要求逐字带回)、
    /// Bearer 鉴权头带上、tools/tool_choice/max_tokens 在 body 里。
    #[tokio::test]
    async fn step_cloud_renders_turns_and_auth_into_request() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": "好"}, "finish_reason": "stop"}]
        })
        .to_string();
        let (port, rx) = spawn_http_once("200 OK", body).await;
        let raw_replay = json!({
            "role": "assistant",
            "reasoning_content": "思考过程",
            "tool_calls": [{"id": "cR", "type": "function",
                "function": {"name": "get_timeline", "arguments": "{}"}}]
        });
        let turns = vec![
            Turn::User("昨天下午在干嘛".into()),
            Turn::AssistantCall {
                id: "cR".into(),
                name: "get_timeline".into(),
                args: "{}".into(),
                raw: Some(raw_replay.clone()),
            },
            Turn::ToolResult {
                id: "cR".into(),
                content: "时间线结果".into(),
            },
            Turn::AssistantText("初步答案".into()),
            Turn::AssistantCall {
                id: "c9".into(),
                name: "search_text".into(),
                args: "{\"keywords\":[\"报销\"]}".into(),
                raw: None,
            },
            Turn::ToolResult {
                id: "c9".into(),
                content: "搜索结果".into(),
            },
        ];
        cloud_client(port).step("系统人设", &turns).await.unwrap();

        let req = rx.await.unwrap();
        let head_end = req.find("\r\n\r\n").unwrap();
        let head = req[..head_end].to_lowercase();
        assert!(
            head.starts_with("post /v1/chat/completions"),
            "尾斜杠应被 trim: {head}"
        );
        assert!(
            head.contains("authorization: bearer sk-test"),
            "api_key 非空必须带 Bearer 头"
        );
        let body: Value = serde_json::from_str(&req[head_end + 4..]).unwrap();
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["tool_choice"], "auto");
        // Auto 也给足思考预算:DeepSeek 在 Auto 下默认思考,1024 会被
        // 思考链打满、正文截空(实弹见 scripts/llm/thinking_probe.py)。
        // 字段名必须是 max_completion_tokens:OpenAI 对 max_tokens 直接 400
        assert_eq!(body["max_completion_tokens"], 12288);
        assert!(
            body.get("max_tokens").is_none(),
            "云端不得再发旧字段名(OpenAI 会 400)"
        );
        assert_eq!(
            body["tools"].as_array().unwrap().len(),
            3,
            "三个工具定义应随请求下发"
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 7, "system + 6 条 turn");
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "系统人设");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[2], raw_replay, "带 raw 的 AssistantCall 必须逐字回放");
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "cR");
        assert_eq!(msgs[3]["content"], "时间线结果");
        assert_eq!(msgs[4]["role"], "assistant");
        assert_eq!(msgs[4]["content"], "初步答案");
        assert_eq!(
            msgs[5]["tool_calls"][0]["id"], "c9",
            "无 raw 的 call 按 id/name/args 重构"
        );
        assert_eq!(msgs[5]["tool_calls"][0]["function"]["name"], "search_text");
        assert_eq!(msgs[6]["tool_call_id"], "c9");
    }

    /// 401:状态码与服务端错误说明都要出现在错误信息里——用户配错 key 时
    /// 必须能从提示里直接看出"鉴权失败 + 服务端原话"。
    #[tokio::test]
    async fn step_cloud_401_error_carries_status_and_server_message() {
        let (port, _rx) = spawn_http_once(
            "401 Unauthorized",
            r#"{"error":{"message":"Invalid API key provided"}}"#.into(),
        )
        .await;
        let err = cloud_client(port)
            .step("s", &[Turn::User("q".into())])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("401"), "缺状态码: {err}");
        assert!(
            err.contains("Invalid API key provided"),
            "缺服务端原话: {err}"
        );
    }

    /// 429/500 同理透传;chat 通路对 429 不做退避重试(交互式场景等不起),
    /// 应立刻把限流信息报给用户。
    #[tokio::test]
    async fn step_cloud_429_and_500_pass_through_readably() {
        let (port, _rx) = spawn_http_once(
            "429 Too Many Requests",
            r#"{"error":{"message":"rate limited"}}"#.into(),
        )
        .await;
        let err = cloud_client(port)
            .step("s", &[Turn::User("q".into())])
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("429") && err.contains("rate limited"),
            "429 信息不可读: {err}"
        );

        let (port, _rx) =
            spawn_http_once("500 Internal Server Error", "上游模型服务不可用".into()).await;
        let err = cloud_client(port)
            .step("s", &[Turn::User("q".into())])
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("500") && err.contains("上游模型服务不可用"),
            "500 信息不可读: {err}"
        );
    }

    /// 非 2xx 的长错误体预览按**字符**截到 300、状态码保留——报错不能无限长,
    /// 且中文等多字节错误体(网关中文错误页)不得因切在字符中间而 panic。
    /// 历史:旧实现按字节切片,第 300 字节落在多字节字符中间会崩,B 档测试实锤后修复。
    #[tokio::test]
    async fn step_cloud_long_error_body_preview_truncates_on_char_boundary() {
        // ASCII 契约:恰好保留前 300 个字符
        let body = "a".repeat(400);
        let (port, _rx) = spawn_http_once("500 Internal Server Error", body).await;
        let err = cloud_client(port)
            .step("s", &[Turn::User("q".into())])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("500"), "应报 500: {err}");
        assert!(
            err.contains(&"a".repeat(300)) && !err.contains(&"a".repeat(301)),
            "预览应恰为前 300 字符: {err}"
        );

        // 多字节回归:299 个 ASCII + 中文,旧实现在此 panic,现应正常截断
        let tricky = format!("{}错误详情继续", "x".repeat(299));
        let (port, _rx) = spawn_http_once("500 Internal Server Error", tricky).await;
        let err = cloud_client(port)
            .step("s", &[Turn::User("q".into())])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("500"), "多字节体也应正常报 500: {err}");
        assert!(err.contains("错"), "第 300 个字符(错)应被保留: {err}");
        assert!(!err.contains("误"), "第 301 个字符起应被截断: {err}");
    }

    /// 2xx 但响应体不是 JSON(网关吐 HTML 错误页等):应报"响应不是 JSON"
    /// 而不是 panic 或含糊的空内容错误。
    #[tokio::test]
    async fn step_cloud_non_json_body_reports_parse_error() {
        let (port, _rx) = spawn_http_once("200 OK", "<html>Bad Gateway</html>".into()).await;
        let err = cloud_client(port)
            .step("s", &[Turn::User("q".into())])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("响应不是 JSON"), "畸形体应报解析错误: {err}");
    }

    /// 缺字段容错:choices 整个缺失,以及 content 只有空白——都应归为
    /// "模型返回空内容"这一个用户可懂的错误,而不是 panic / unwrap 崩溃。
    #[tokio::test]
    async fn step_cloud_missing_choices_or_blank_content_reports_empty() {
        for body in [json!({}).to_string(), json!({
            "choices": [{"message": {"role": "assistant", "content": "   "}, "finish_reason": "stop"}]
        })
        .to_string()]
        {
            let (port, _rx) = spawn_http_once("200 OK", body.clone()).await;
            let err = cloud_client(port)
                .step("s", &[Turn::User("q".into())])
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("模型返回空内容"), "body={body} 应报空内容: {err}");
        }
    }

    /// 拒连(端口上没有服务):应映射为带"请求失败"前缀的错误——
    /// 这是"云端地址配错/断网"时用户看到的第一行字。
    #[tokio::test]
    async fn step_cloud_connection_refused_maps_to_request_error() {
        let port = free_local_port();
        let err = cloud_client(port)
            .step("s", &[Turn::User("q".into())])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("请求失败"), "拒连应报请求失败: {err}");
    }

    /// 挂死不回包的服务:请求应保持等待(靠 120s 客户端超时兜底),
    /// 绝不能早退成假成功/假错误。这里断言 500ms 内仍 pending;
    /// 真实 120s 超时触发需要 tokio test-util 虚拟时钟,本 crate 未启用,不硬测。
    #[tokio::test]
    async fn step_cloud_hanging_server_stays_pending() {
        // backlog 会完成 TCP 握手但应用层永不 accept/回包——最像"服务活着但卡死"
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = cloud_client(port);
        let r = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            client.step("s", &[Turn::User("q".into())]),
        )
        .await;
        assert!(
            r.is_err(),
            "不回包时 500ms 内不应有任何结果(等超时兜底),实际: {r:?}"
        );
        drop(listener);
    }

    // ---- 以下测 ai::llm::ExternalChatClient 的空回复归因(云端文本通路的
    // [LLM_EMPTY_*] 错误码,前端按码显示本地化解释,产生条件是对外契约)。
    // 只调用其公开 API,不改动 ai/llm.rs。----

    async fn external_chat_err(body: Value) -> String {
        let (port, _rx) = spawn_http_once("200 OK", body.to_string()).await;
        crate::ai::llm::ExternalChatClient::new(
            &format!("http://127.0.0.1:{port}/v1"),
            "m".into(),
            String::new(),
            4096,
        )
        .unwrap()
        .chat_text("s", "u", &[])
        .await
        .unwrap_err()
        .to_string()
    }

    /// reasoning_content 非空 + content 空 → 思考链吃光了输出预算,
    /// 归因 REASONING——即使 finish_reason/usage 同时满足别的码,思考链证据优先。
    #[tokio::test]
    async fn external_empty_with_reasoning_is_llm_empty_reasoning() {
        let err = external_chat_err(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "", "reasoning_content": "让我想想…(长思考链)"},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 800, "completion_tokens": 512}
        }))
        .await;
        assert!(
            err.contains("[LLM_EMPTY_REASONING]"),
            "应归因思考链占满: {err}"
        );
    }

    /// finish_reason=stop + completion_tokens=0 + 无思考链 → prompt 一进去就 EOS
    /// (chat template / 模型错配的典型征兆),归因 EOS。
    #[tokio::test]
    async fn external_empty_stop_zero_tokens_is_llm_empty_eos() {
        let err = external_chat_err(json!({
            "choices": [{
                "message": {"role": "assistant", "content": ""},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 800, "completion_tokens": 0}
        }))
        .await;
        assert!(err.contains("[LLM_EMPTY_EOS]"), "应归因即时 EOS: {err}");
    }

    /// finish_reason=length + completion_tokens>0 + content 空 → 生成了 token 却
    /// 没落进 content(老版 llama-server 思考链塞 content 被截),归因 TRUNCATED。
    #[tokio::test]
    async fn external_empty_length_with_tokens_is_llm_empty_truncated() {
        let err = external_chat_err(json!({
            "choices": [{
                "message": {"role": "assistant", "content": ""},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 800, "completion_tokens": 300}
        }))
        .await;
        assert!(err.contains("[LLM_EMPTY_TRUNCATED]"), "应归因截断: {err}");
    }

    /// 兜底:没有思考链、finish_reason=stop 但 usage 缺失(部分服务不返)——
    /// 无法归因到具体成因时给未分类码 [LLM_EMPTY],不能错挂到 EOS/TRUNCATED 上。
    #[tokio::test]
    async fn external_empty_unclassifiable_is_plain_llm_empty() {
        let err = external_chat_err(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "  "},
                "finish_reason": "stop"
            }]
        }))
        .await;
        assert!(err.contains("[LLM_EMPTY]"), "缺 usage 应落未分类码: {err}");
        assert!(!err.contains("[LLM_EMPTY_EOS]"), "不能误判成 EOS: {err}");
    }
}
