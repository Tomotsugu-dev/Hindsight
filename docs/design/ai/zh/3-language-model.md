# LanguageModel：调模型的接口

> **状态**：草稿。总体结构见 [AI 后端架构](0-ai-architecture.md)。

所有模型都通过这一个接口调用。模型由 `LanguageModelProvider` 打开（见[模型选择和配置](3-settings.md#22-后端languagemodelprovider)）。本文定义请求、结果、能力描述和错误，并写明每一项的含义。各家怎么翻译，见[适配规格](4-adapters.md)。

---

## 1. 类型

```rust
/// 每种模型都要实现的函数。
pub trait LanguageModelOps {
    fn capabilities(&self) -> &Capabilities;
    /// 估算一段文字有多少 token。ReportContext 用它按预算裁剪。
    fn estimate_tokens(&self, text: &str) -> u32;
    async fn generate(&self, req: Request) -> Result<Response, ModelError>;
}

/// 打开以后的模型。方法拿到的就是它。
pub enum LanguageModel {
    ChatCompletions(ChatCompletionsModel), // 云端各家和本机 llama-server。本机的带着 EngineGuard
    AppleFm(AppleFmModel),
    #[cfg(test)]
    Fake(FakeModel),                       // 按脚本回复
}

pub struct Request {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,          // 空 = 不给工具
    pub output: Output,                // Text | Json(schema)
    pub reasoning: Reasoning,
    pub max_output_tokens: u32,        // 正文上限，不含思考
    pub temperature: Option<f32>,      // 任务的建议值
}

pub enum Message {
    User { text: String, images: Vec<ImageRef> },
    Assistant { text: String, call: Option<ToolCall>, carry: Option<Carry> },
    ToolResult { call_id: String, content: String },   // call_id 就是 ToolCall.id
}

pub enum Reasoning { Default, Off, Low, Medium, High, Max }

pub struct Response {
    pub text: String,
    pub call: Option<ToolCall>,        // 最多一个
    pub carry: Option<Carry>,          // 下一步要原样带回的内容
    pub applied: Reasoning,            // 实际生效的思考档位
    pub usage: Usage,
}

pub struct Usage {
    pub input_tokens: Option<u64>,     // 模型没报就是 None
    pub output_tokens: Option<u64>,    // 含思考
    pub reasoning_tokens: Option<u64>,
    pub elapsed_ms: u64,
}

/// 告诉模型有哪个工具、怎么调。由 Tools 生成，适配器翻成各家的格式。
pub struct ToolSpec {
    pub name: String,                  // 比如 "get_timeline"
    pub description: String,           // 给模型看的说明，按提示词语言写
    pub parameters: serde_json::Value, // 参数的 JSON Schema
}

/// 模型发来的一次调用。参数还没检查过。
pub struct ToolCall {
    pub id: String,                    // 云端由厂商给，本机由适配器编号
    pub name: String,
    pub arguments: serde_json::Value,
}
```

把 `Response` 的 `text`、`call`、`carry` 原样放进一条 `Message::Assistant`，就是下一步的历史。

---

## 2. 思考

### 档位的含义

| 档位 | 含义 |
|---|---|
| `Default` | 适配器不发任何思考字段。模型按它自己的默认来 |
| `Off` | 明确关掉 |
| `Low` … `Max` | 明确打开，强度依次增加 |

### 三条规则

1. **降级**：模型没有请求的那一档时，取它有的、更高的最近一档；没有更高的，取它的最高档。只有开关的模型，任何强度都等于「开」。
   - 例：DeepSeek 没有 `Medium`，取 `High`。OpenAI 没有 `Max`，取 `High`。
2. **适配器可以改写档位**：请求的其他部分不允许思考时，适配器改成允许的档位，并在 `Response.applied` 里报告。
   - 例：OpenAI 的 Chat Completions 带工具时不能思考，适配器强制成 `Off`。
3. **思考占预算**：见[第 3 节](#3-输出预算)。

### 「自动」不在这一层

用户在界面上看到的「自动」，由自动配置在调用前翻成 `Default` 或 `Off`（见[模型选择和配置](3-settings.md#3-逻辑自动配置)）。适配器只做机械的翻译，不做判断。

---

## 3. 输出预算

输入和输出共用上下文，要先定输出的上限，才算得出输入能放多少（#245）。上限也让停不下来的模型有个头。

`max_output_tokens` 是回答的上限。思考也算在输出里，上限只够回答时，回答会是空的。所以模型可能会思考时（包括 `Default`），适配器发出去的是回答上限和思考底线里大的那个；确定关了思考，就只发回答上限。

- 思考底线：云端 12288，本机 4096（见[适配规格 3.2](4-adapters.md#32-其他项)）。
- 输入能放多少，按发出去的上限算。
- 字段名由适配器定。

---

## 4. 对话历史和原样带回

有些模型要求：工具循环的下一步，必须把上一步的 assistant 消息原样带回，包括思考内容。少了就拒绝请求。

`Carry` 就是放这些内容的地方。

- 适配器在 `Response.carry` 里交出它，上层不读、不改，下一步放回 `Message::Assistant.carry`。
- 只有产生它的那个适配器能读。
- 对话中途换了模型，`carry` 丢掉。适配器按 `text` 和 `call` 重新拼这条消息。

---

## 5. 工具调用

- **一步最多一个调用。** 模型一次返回多个时，适配器只留第一个，`carry` 里也只留第一个。
- **上层不知道工具是怎么实现的。** 能力描述里只有 `tool_use: bool`。用原生工具还是 grammar，打开模型时按模型定，见[适配规格](4-adapters.md#2-chat-completions协议层的规则)。
- 工具的定义、校验、执行不在这里，见 [AI 怎么读数据](3-data-access.md#2-tools)。

---

## 6. 结构化输出

`Output::Json(schema)` 要求模型输出符合 schema 的 JSON。各家能保证的程度不同：

| `structured_output` | 保证什么 |
|---|---|
| `Strict` | 输出一定符合 schema |
| `Loose` | 输出一定是 JSON，字段不保证 |
| `None` | 没有保证，只能靠提示词 |

- 不管哪一种，上层拿到结果后都要自己校验。
- 带工具的请求不能同时要求 `Json`。

---

## 7. 能力描述

```rust
pub struct Capabilities {
    pub on_device: bool,                       // 跑在这台机器上：本机模型、Apple FM
    pub context_length: Option<u32>,           // 上下文长度（token）。None = 不知道
    pub tool_use: bool,
    pub structured_output: StructuredOutput,   // Strict | Loose | None
    pub vision: bool,
    pub reasoning: ReasoningSupport,
}

impl ReasoningSupport {
    /// 这个模型可以选哪些档位。带不带工具，结果可能不同。
    pub fn levels(&self, with_tools: bool) -> Vec<Reasoning>;
}
```

- **按具体的模型定，不按厂商定。** 同一家的不同模型，能力可以不同。
- **每一项的值从哪来**，按这个顺序取第一个有的：
  1. 用户在云端连接里填的
  2. 运行时读到的：Apple FM 的上下文长度，本机引擎启动后报的上下文长度
  3. 测试连接时探测到的
  4. 内置的已知模型表
  5. 不知道
- **不知道时怎么办**：由自动配置给保守的默认值（见[模型选择和配置](3-settings.md#3-逻辑自动配置)）。
- 界面上的思考档位下拉框，内容来自 `levels()`。

---

## 8. 错误

| 错误 | 意思 | 谁处理 |
|---|---|---|
| `Unavailable(原因)` | 系统不支持、模型没下载、引擎没装、没登录 | ① 显示原因 |
| `ContextOverflow` | 输入超过上下文长度 | 待定，见[待决定](#10-待决定) 1 |
| `Refused` | 内容被模型或厂商拦下 | 待定，见[待决定](#10-待决定) 1 |
| `RateLimited` | 重试用完还在限流 | ① 显示 |
| `Auth` | Key 错，或登录过期 | ① 引导去设置 |
| `Empty(code)` | 模型没写出正文，带 `[LLM_EMPTY_*]` 错误码 | 前端按码显示提示 |
| `Transport` | 网络问题 | ① 显示 |
| `Provider(原文)` | 厂商返回的其他错误：模型不存在、服务器错误 | ① 显示原文 |

- 各家的报错怎么对应到这张表，是适配器的事，见[适配规格](4-adapters.md)。
- 限流（429）和断线，在 ⑤ 的模型请求传输里重试。上层只看到最后的结果。

---

## 9. 其他约定

- **取消**：丢弃 `generate` 的 future。HTTP 实现靠断开连接。进程内的实现（Apple FM）在 Drop 时通知底层停下。
- **流式**：只是传输方式，结果一次返回。以后聊天要逐字显示，再加一个方法，不改 `Request`。
- **温度**：`temperature` 是任务的建议值。适配器在厂商不接受时不发这个字段。
- **分发**：每种模型实现 `LanguageModelOps`。`LanguageModel` 这个 enum 把它们收在一起，自己也实现 `LanguageModelOps`：每个函数一个 `match`，转给对应的分支。
  - ② 拿到的是 enum，不用泛型。
  - trait 里有 async 函数时，做成 `dyn` 要加 `async-trait` 依赖。enum 不用。
  - 加一种模型，就加一个分支。插件不能加模型的实现。
  - `LanguageModelProvider`、`ReportContext` 也这样写。

---

## 10. 待决定

1. 输入超过上下文长度时怎么办？
