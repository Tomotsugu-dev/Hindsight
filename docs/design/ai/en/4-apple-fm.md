# Apple FM

> **Status**: Draft. Only its place in the architecture is decided. The detailed design will be written when it follows the process for a new feature. For the overall structure, see [AI backend architecture](0-ai-architecture.md).
>
> **Related**: #247

Apple Foundation Models is the local language model that comes with macOS. The system manages it. The user does not need to download a model. The user does not need to start Ollama. The user does not need an API key.

---

## 1. Its place in the architecture

It is one implementation of ④. It calls the system framework inside the process. It does not use HTTP. It is compiled only for macOS. It does not affect Windows.

It implements `LanguageModelProvider`. There is only one model under it. After it is opened, it returns a `LanguageModel`.

**`LanguageModel`** capability description:

| Item | Value |
|---|---|
| `on_device` | `true` |
| `context_length` | Read at runtime. #247 says the length is 4,096 on macOS 26. On some macOS 27 systems it is 8,192. Not verified |
| `tool_use` | `false` (first version) |
| `structured_output` | `None` (first version) |
| `vision` | `false` |
| `reasoning` | No levels |

The context is short. So the daily report can only use single generation. The material is also trimmed more.

**`LanguageModelProvider`** methods:

| Method | What it does |
|---|---|
| `list` | Only one item |
| `availability` | Ask the system. Reasons it is not available: the chip does not support it, the system is too old, Apple Intelligence is not turned on, or the model has not been downloaded yet |
| `describe` | Read the context length at runtime |
| `probe` | Generate one sentence |
| `open` | No preparation is needed. It returns the model directly |

---

## 2. What the adapter needs to do

How each item of `Request` is done (see [LanguageModel](3-language-model.md#1-types)):

| What to translate | How Apple FM does it |
|---|---|
| Messages | Each generation creates a new session. Between segments, the previous conversation is not included |
| Thinking | None. Any level that is received is ignored. `effective_reasoning = Off` |
| Output budget | Give it to the system's generation options |
| Tools, `carry`, and structured output | Not in the first version |
| Transport | A small Swift library exports C functions. Rust calls them through FFI. It does not stream |
| Cancel | When the Rust future is discarded, tell Swift to stop |
| Errors | See below |
| Token estimate | See [To be decided](#3-to-be-decided) 4 |

**guardrail**: Text output uses `permissiveContentTransformations`. Apple's docs say it is suitable for summarizing text that may contain sensitive content. It takes effect only on plain text output.

**How to recognize errors**:

| What the system reports | Unified error |
|---|---|
| Not available | `Unavailable(reason)` |
| Exceeds the context length | `ContextOverflow` |
| Blocked by the guardrail | `Refused` |

Estimate the tokens before sending. If the estimate exceeds the context length, report `ContextOverflow` directly. Do not give it to the system. The reason is a real test in the #247 comments. When the input is much greater than the context length, the system reports "may contain unsafe content". If it is read literally, it would be recognized as `Refused`.

Real test in the #247 comments (macOS 27.0.1, one day of real data):

- `fm license` is not needed.
- The guardrail was not triggered.
- Each segment takes 4–16 seconds. It runs mainly on the Neural Engine. It uses very little power.

---

## 3. To be decided

1. Which way to bridge Swift (write our own `@_cdecl` exports, `swift-bridge`, or `swift-rs`)? What is the effect on the build and on CI?
2. Does it affect code signing and notarization? Is an extra entitlement needed? #247 mentioned this. It has not been checked yet.
3. What is the minimum macOS version to support?
4. How to estimate tokens: does the system provide a way to count tokens? If not, estimate from the characters, the same way as the cloud.
