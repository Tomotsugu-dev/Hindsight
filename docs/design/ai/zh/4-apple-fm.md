# Apple FM

> **状态**：草稿。只定了它在架构里的位置，详细设计走新功能流程时再写。总体结构见 [AI 后端架构](0-ai-architecture.md)。
>
> **相关**：#247

Apple Foundation Models 是 macOS 自带的本机语言模型，由系统管理。用户不用下载模型，不用开 Ollama，也不用 API Key。

---

## 1. 在架构里的位置

它是 ④ 的一个实现，在进程内调系统框架，不走 HTTP。只在 macOS 编译，不影响 Windows。

它实现 `LanguageModelProvider`，下面只有一个模型。打开后交出一个 `LanguageModel`。

**`LanguageModel`** 的能力描述：

| 项 | 值 |
|---|---|
| `on_device` | `true` |
| `context_length` | 运行时读。#247 说 macOS 26 是 4,096，部分 macOS 27 是 8,192，未核实 |
| `tool_use` | `false`（第一版） |
| `structured_output` | `None`（第一版） |
| `vision` | `false` |
| `reasoning` | 没有档位 |

上下文短，所以日报只能用单次生成，材料也会裁得更多。

**`LanguageModelProvider`** 的方法：

| 方法 | 做什么 |
|---|---|
| `list` | 只有一项 |
| `availability` | 问系统。不可用的原因：芯片不支持、系统太旧、没开 Apple Intelligence、模型还没下载 |
| `describe` | 运行时读上下文长度 |
| `probe` | 生成一句话 |
| `open` | 不用准备，直接交出模型 |

---

## 2. 适配器要做的事

`Request` 的每一项怎么做（见 [LanguageModel](3-language-model.md#1-类型)）：

| 要翻译的地方 | Apple FM 怎么做 |
|---|---|
| 消息 | 每次生成新建一个 session，段和段之间不带上一次的对话 |
| 思考 | 没有。收到任何档位都忽略，`effective_reasoning = Off` |
| 输出预算 | 交给系统的生成选项 |
| 工具、`carry`、结构化输出 | 第一版没有 |
| 传输 | 一个小 Swift 库导出 C 函数，Rust 通过 FFI 调用。不流式 |
| 取消 | Rust 的 future 被丢弃时，通知 Swift 停下 |
| 错误 | 见下 |
| token 估算 | 见[待决定](#3-待决定) 4 |

**guardrail**：文字输出用 `permissiveContentTransformations`。Apple 的文档说它适合给可能含敏感内容的文字做摘要。它只对纯文字输出生效。

**错误怎么认**：

| 系统报的 | 统一错误 |
|---|---|
| 不可用 | `Unavailable(原因)` |
| 超过上下文长度 | `ContextOverflow` |
| guardrail 拦下 | `Refused` |

发之前先估算 token。超过上下文长度就直接报 `ContextOverflow`，不交给系统。原因是 #247 评论里的实测：输入远超上下文长度时，系统报的是「可能含不安全内容」，照字面会被认成 `Refused`。

#247 评论里的实测（macOS 27.0.1，一天的真实数据）：

- 不需要 `fm license`
- 没有触发 guardrail
- 每段 4–16 秒，主要跑在 Neural Engine 上，耗电很低

---

## 3. 待决定

1. 用哪种方式桥接 Swift（自己写 `@_cdecl` 导出、`swift-bridge`、`swift-rs`）？对构建和 CI 有什么影响？
2. 对签名和公证有没有影响？要不要额外的 entitlement？#247 提了，还没查。
3. 最低支持哪个 macOS 版本？
4. token 怎么估：系统有没有提供数 token 的办法？没有的话，用和云端一样的按字符估算。
