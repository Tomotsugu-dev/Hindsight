# 适配规格

> **状态**：草稿。总体结构见 [AI 后端架构](0-ai-architecture.md)。接口的含义见 [LanguageModel](3-language-model.md)。

本文写明：统一的请求发给每一家时变成什么，每一家的响应和报错怎么变回统一的结果。

带 `*` 的是按厂商文档或经验写的，还没用真实响应核对，实现时要核对。

---

## 1. 两个适配器

- **Chat Completions**：云端各家和本机的 llama-server 共用这一个，因为它们说的都是这个协议。各家不一样的地方写在方言表里。方言有 openai、deepseek、kimi、openrouter、通用、llama-server。
- **Apple FM**：在进程内调系统框架，不走 HTTP。见 [Apple FM](4-apple-fm.md)。

鉴权不归适配器管。应用公共的凭证给出请求头（见[架构 3.6](0-ai-architecture.md#36-应用公共部分)），用的是 API Key 还是登录（#149），适配器分不出来。

方言表解决不了的，以后各加一个适配器：OpenAI 带工具又要思考（Chat Completions 不允许，要用 Responses API），Anthropic、Gemini 自己的协议。`LanguageModel` 接口不变。

---

## 2. Chat Completions：协议层的规则

所有方言共用。

**消息**

- `system` 是第一条消息。
- 只有文字时，`content` 发字符串，有的厂商不收数组。带图片时发数组，图片用 data URI。

**工具**

打开模型时，按模型选一种做法：

```rust
enum ToolProtocol {
    Native,   // 请求带 tools，模型回 tool_calls
    Grammar,  // 工具说明写进 instructions，grammar 让模型只能输出决策对象
}
```

- 云端各家：`Native`。
- llama-server：模型的对话模板支持工具调用，就用 `Native`，引擎启动时要加 `--jinja`。不支持就用 `Grammar`。
- 小模型走 `Native` 时格式会不会坏，要实测。坏得多，就再加一条门槛，比如参数量。
- `Native`：请求带 `tools` 和 `tool_choice: "auto"`。工具结果用 `role: "tool"` 和 `tool_call_id`。`carry` 是整条 assistant 消息。
- `Grammar`：请求带 `json_schema`，输出只能是决策对象：`action` 是某个工具名，或者是 `answer`。整段历史渲染成一条 user 消息。

**流式**

- 收到 `[DONE]` 才算完整。没收到就断了，报 `Transport`。
- 解析不了的块跳过。
- 不看 `finish_reason`，有的厂商在流里给 `null`。

---

## 3. Chat Completions：方言表

### 3.1 思考

`Default` 一律不发思考字段，例外见最后一列。没有的档位怎么降，见 [LanguageModel 第 2 节](3-language-model.md#2-思考)。

| 方言 | `Off` 发什么 | 强度发什么 | 有哪些档 | 特别规则 |
|---|---|---|---|---|
| openai | `reasoning_effort: "none"` | `reasoning_effort: "<档>"` | low、medium、high | 带工具时一律发 `none`，`Default` 也是。`effective_reasoning = Off` |
| kimi | 同 openai | 同 openai | low、medium、high | 个别模型强制思考，发 `none` 会被拒，靠自愈撤掉 |
| deepseek | `thinking: {type: "disabled"}` | `thinking: {type: "enabled"}`，加 `reasoning_effort: "<档>"` | low、high、max | — |
| openrouter | `reasoning: {enabled: false}` | `reasoning: {effort: "<档>"}` | low、medium、high | 不能同时发顶层的 `reasoning_effort` |
| 通用 | `chat_template_kwargs: {enable_thinking: false}` | `chat_template_kwargs: {enable_thinking: true}` | 只有开关 | 不认这个字段的端点会忽略它 |
| llama-server | 同通用 | 同通用 | 只有开关 | 收到 `Default` 按 `Off` 发：本机模型默认思考会吃光预算 |

### 3.2 其他项

| 方言 | 预算字段 | 思考底线 | 流式 | 温度 | 超时 |
|---|---|---|---|---|---|
| 云端各家 | `max_completion_tokens` | 12288 | 开 | 不发 | 建连 30 秒；两块数据之间 60 秒；不限总时长 |
| llama-server | `max_tokens` | 4096 | 关 | 发 | 总时长 600 秒 |

- 云端必须开流式：不开的话，写完之前连接上没有数据，长回答会被网络中途掐断。
- 云端不发温度：有的模型只接受固定值，用厂商的默认值最安全。
- 思考底线的依据：云端实测 DeepSeek 最高档思考 6K token 以上；本机实测完整的思考链 1K 到 2K token。

### 3.3 重试

| 情况 | 云端 | llama-server |
|---|---|---|
| 限流（429） | 最多 5 次。每次等 `Retry-After` 和 2、4、8、16、32 秒里较长的那个 | 不会发生 |
| 断线 | 最多 2 次，分别等 2 秒、4 秒 | 不重试 |
| 400 | 自愈，最多 3 轮 | 不自愈：是我们自己发错了参数 |

### 3.4 自愈

云端的 400 大多是某个参数不合这个模型，报错文字里通常写了怎么改。自愈就是按报错改请求，再发一次。规则不分方言：

| 报错的意思 | 怎么改 |
|---|---|
| 这个参数不支持，请用另一个名字 | 改名 |
| 这个值不支持 | 换成报错里列出的第一个合法值。没列就删掉这个字段 |
| 这两样不能一起用 | 把被点名的参数改成安全值。是 `reasoning_effort` 就改成 `none`，原来没发也要补上 |
| 这个模型必须思考 | 撤掉所有思考字段 |
| 不认识这个参数 | 删掉 |

- `model`、`messages`、`tools`、`tool_choice` 永远不删。删了请求能过，功能已经坏了。
- 没有规则能改，就直接报错。
- 自愈成功后记住这次的改法，同一个连接下次直接照改。记在哪见[待决定](#4-待决定) 2。

### 3.5 错误怎么认

| 统一错误 | 怎么认 |
|---|---|
| `Auth` | HTTP 401、403 |
| `RateLimited` | 重试用完还是 429 |
| `ContextOverflow` | HTTP 400，错误码是 `context_length_exceeded`*，或报错文字提到超过上下文长度*。llama-server 只看报错文字* |
| `Refused` | 结束原因是 `content_filter`* |
| `Empty(code)` | 正文为空。只有思考：`LLM_EMPTY_REASONING`；正常结束但没有输出：`LLM_EMPTY_EOS`；被长度截断：`LLM_EMPTY_TRUNCATED`；其他：`LLM_EMPTY` |
| `Transport` | 连不上；断流；重试用完 |
| `Unavailable` | llama-server：引擎没装，模型文件不见了 |
| `Provider(原文)` | 其他：模型不存在、自愈不了的 400、服务器错误 |

### 3.6 token 估算和能力

| 方言 | `estimate_tokens` | `context_length` | `tool_use` | `vision` |
|---|---|---|---|---|
| 云端各家 | 按字符估：中日韩文字一个字算一个，其他四个字符算一个，再加两成余量。系数实测后定 | 内置的已知模型表；没有就是不知道 | 默认是；测试连接时探测 | 测试连接时探测 |
| llama-server | 调引擎的 `/tokenize`* | 引擎启动后从 `/props` 读* | 是。模板支持工具就走 `Native`，不支持走 `Grammar` | 有 mmproj 时是 |

`structured_output`：llama-server 是 `Strict`（grammar）。云端各家先按 `None` 处理，Agent 流水线设计时逐家核对。

---

## 4. 待决定

1. llama-server 并进 Chat Completions 适配器，作为一种方言。同意吗？另一种是给它单独一个适配器。
2. 记住的修正放在哪：只放内存（重启后重新学），还是存进这个连接的设置里？
3. token 估算的系数怎么定：先用上面的粗略值，还是先实测几家再写？
4. 传输层定义的 trait 要回答哪些问题？已知两个：流怎么拼、完不完整；400 怎么改请求。还没定：过载的状态码（比如 Anthropic 的 529）由 trait 认，还是写进 `TransportPolicy`。设计时对着 Anthropic、Gemini 的格式核一遍，确认加这两家时 trait 不用改。
