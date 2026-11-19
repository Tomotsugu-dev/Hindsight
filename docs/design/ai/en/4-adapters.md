# Adapter specification

> **Status**: Draft. For the overall structure, see [AI backend architecture](0-ai-architecture.md). For what the interface means, see [LanguageModel](3-language-model.md).

This document says what a unified request becomes when it is sent to each provider. It also says how each provider's response and error turn back into a unified result.

Items with `*` are written from provider docs or from experience. They have not been checked against real responses. Check them when you implement this.

---

## 1. Two adapters

- **Chat Completions**: Cloud providers and the local llama-server share this one adapter, because they all use this protocol. The differences are written in the dialect table. The dialects are openai, deepseek, kimi, openrouter, generic, and llama-server.
- **Apple FM**: It calls the system framework inside the process. It does not use HTTP. See [Apple FM](4-apple-fm.md).

Authentication is not the adapter's job. The app's shared credentials give the request headers (see [Architecture 3.6](0-ai-architecture.md#36-common-parts-of-the-app)). The adapter cannot tell an API key from a login (#149).

What the dialect table cannot handle will each get an adapter later: OpenAI with tools and thinking together (Chat Completions does not allow that, so use the Responses API), and Anthropic and Gemini with their own protocols. The `LanguageModel` interface does not change.

---

## 2. Chat Completions: protocol-layer rules

All dialects share these rules.

**Messages**

- `system` is the first message.
- When there is only text, send `content` as a string. Some providers do not accept an array. When there are images, send an array. Images use data URIs.

**Tools**

When a model is opened, pick one way for that model:

```rust
enum ToolProtocol {
    Native,   // The request includes tools. The model returns tool_calls.
    Grammar,  // The tool description is written into instructions. The grammar lets the model output only a decision object.
}
```

- Cloud providers: `Native`.
- llama-server: if the model's chat template supports tool calls, use `Native`. Add `--jinja` when the engine starts. If it does not support tool calls, use `Grammar`.
- Test for real whether small models break the format when they use `Native`. If they break it often, add another threshold, such as the number of parameters.
- `Native`: the request includes `tools` and `tool_choice: "auto"`. Tool results use `role: "tool"` and `tool_call_id`. `carry` is the whole assistant message.
- `Grammar`: the request includes `json_schema`. The output can only be a decision object: `action` is a tool name, or it is `answer`. The whole history is turned into one user message.

**Streaming**

- The response is complete only when `[DONE]` arrives. If the stream ends before that, report `Transport`.
- Skip chunks that cannot be parsed.
- Do not look at `finish_reason`. Some providers send `null` for it in the stream.

---

## 3. Chat Completions: dialect table

### 3.1 Thinking

For `Default`, never send thinking fields. Exceptions are in the last column. For how to downgrade a level the model does not have, see [LanguageModel section 2](3-language-model.md#2-thinking).

| Dialect | What `Off` sends | What a strength sends | Levels it has | Special rules |
|---|---|---|---|---|
| openai | `reasoning_effort: "none"` | `reasoning_effort: "<level>"` | low, medium, high | With tools, always send `none`. `Default` does this too. `effective_reasoning = Off` |
| kimi | Same as openai | Same as openai | low, medium, high | A few models force thinking. Sending `none` is rejected. Self-heal removes the thinking fields |
| deepseek | `thinking: {type: "disabled"}` | `thinking: {type: "enabled"}`, plus `reasoning_effort: "<level>"` | low, high, max | — |
| openrouter | `reasoning: {enabled: false}` | `reasoning: {effort: "<level>"}` | low, medium, high | Do not send a top-level `reasoning_effort` at the same time |
| generic | `chat_template_kwargs: {enable_thinking: false}` | `chat_template_kwargs: {enable_thinking: true}` | Only on and off | An endpoint that does not recognize this field ignores it |
| llama-server | Same as generic | Same as generic | Only on and off | When `Default` is received, send `Off`. A local model thinks by default, and that uses up the whole budget |

### 3.2 Other items

| Dialect | Budget field | Thinking minimum | Streaming | Temperature | Timeout |
|---|---|---|---|---|---|
| Cloud providers | `max_completion_tokens` | 12288 | On | Do not send | 30 seconds to connect; 60 seconds between two pieces of data; no limit on total time |
| llama-server | `max_tokens` | 4096 | Off | Send | 600 seconds total |

- Cloud providers must turn streaming on. If it is off, the connection has no data before the answer is written. A long answer is then stopped by the network in the middle.
- Cloud providers do not send temperature. Some models accept only a fixed value. The provider's default is the safest.
- The thinking minimum comes from real tests. On the cloud, DeepSeek at the highest level uses more than 6K tokens of thinking. On a local model, a full thinking chain uses 1K to 2K tokens.

### 3.3 Retries

| Case | Cloud | llama-server |
|---|---|---|
| Rate limit (429) | At most 5 times. Each wait is the longer of `Retry-After` and 2, 4, 8, 16, 32 seconds | Will not happen |
| Disconnect | At most 2 times, waiting 2 seconds and then 4 seconds | Do not retry |
| 400 | Self-heal, at most 3 rounds | Do not self-heal: we ourselves sent the wrong parameters |

### 3.4 Self-heal

Most cloud 400 errors mean a parameter does not fit this model. The error text usually says how to change it. Self-heal changes the request as the error says, then sends it again. The rules are the same for every dialect:

| What the error means | How to change it |
|---|---|
| This parameter is not supported. Use another name | Rename it |
| This value is not supported | Use the first valid value listed in the error. If none is listed, delete this field |
| These two cannot be used together | Set the named parameter to a safe value. If it is `reasoning_effort`, set it to `none`. If it was not sent, add it too |
| This model must think | Remove all thinking fields |
| This parameter is not recognized | Delete it |

- Never delete `model`, `messages`, `tools`, or `tool_choice`. If they are deleted, the request can succeed, but the feature is already broken.
- If no rule can make the change, report the error directly.
- After a self-heal succeeds, remember this change. Next time, the same connection applies the change directly. For where it is stored, see [To be decided](#4-to-be-decided) 2.

### 3.5 How to recognize errors

| Unified error | How to recognize it |
|---|---|
| `Auth` | HTTP 401 or 403 |
| `RateLimited` | Still 429 after the retries are used up |
| `ContextOverflow` | HTTP 400, and the error code is `context_length_exceeded`*, or the error text mentions exceeding the context length*. llama-server looks only at the error text* |
| `Refused` | The finish reason is `content_filter`* |
| `Empty(code)` | The main text is empty. Only thinking: `LLM_EMPTY_REASONING`. Ended normally, but no output: `LLM_EMPTY_EOS`. Cut short by the length limit: `LLM_EMPTY_TRUNCATED`. Other: `LLM_EMPTY` |
| `Transport` | Cannot connect; the stream broke; the retries are used up |
| `Unavailable` | llama-server: the engine is not installed, or the model file is missing |
| `Provider(original text)` | Other: the model does not exist, a 400 that self-heal cannot fix, or a server error |

### 3.6 Token estimate and capabilities

| Dialect | `estimate_tokens` | `context_length` | `tool_use` | `vision` |
|---|---|---|---|---|
| Cloud providers | Estimate from the characters. One Chinese, Japanese, or Korean character counts as one token. Four other characters count as one token. Then add 20% extra. The factors will be set after real tests | The built-in table of known models. If the model is not in it, the length is unknown | The default is yes. Probe it when testing the connection | Probe it when testing the connection |
| llama-server | Call the engine's `/tokenize`* | After the engine starts, read it from `/props`* | Yes. If the template supports tools, use `Native`. If it does not, use `Grammar` | Yes when there is an mmproj |

`structured_output`: llama-server is `Strict` (grammar). Treat cloud providers as `None` for now. When the Agent pipeline is designed, check each provider.

---

## 4. To be decided

1. Put llama-server into the Chat Completions adapter, as one dialect. Do you agree? The other option is a separate adapter for it.
2. Where are the remembered corrections stored: only in memory (learn them again after a restart), or in this connection's settings?
3. How should the token-estimate factors be set: use the rough values above first, or test a few providers first and then write them?
4. Which questions must the trait defined by the transport layer answer? Two are known: how to join the stream and whether it is complete; how a 400 changes the request. Not decided yet: overload status codes (for example, Anthropic's 529) are recognized by the trait, or they are written into `TransportPolicy`. During design, check the formats of Anthropic and Gemini. Confirm that the trait does not need to change when these two providers are added.
