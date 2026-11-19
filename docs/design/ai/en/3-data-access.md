# How AI reads data

> **Status**: Draft. For the overall structure, see [AI backend architecture](0-ai-architecture.md).

The model cannot see the database. We turn the user's data into text. We give the text to the model. There are two ways to give it:

- `ReportContext` gives the material before the model is called. Our code decides what to give. For example, the daily report gives the activity timeline for a period of time. The daily report, the weekly report, and the MCP server use it.
- `Tools` gives query results while the model is answering. The model decides what to query. For example, the model wants to see the full timeline of a certain hour. It calls a tool once. Chat, summary + tool loop, and the MCP server use it.

Both ways go through `AiDataPolicy` first. It decides which data must not be given to the model.

---

## 1. ReportContext

In the daily report and the weekly report, text for the model to read is put together in advance.

```rust
/// Functions that the material for every kind of report must implement.
pub trait ReportContextOps {
    /// Which period of time the material is for.
    fn range(&self) -> TimeRange;
    /// Provides the material, no more than max_tokens. Uses model to count tokens.
    async fn material(&self, max_tokens: u32, model: &LanguageModel) -> Result<String>;
}

/// One branch for each kind of report.
pub enum ReportContext {
    Daily(DailyContext),    // One segment of a day
    Weekly(WeeklyContext),  // One week
    #[cfg(test)]
    Fixed(String),          // For tests. Returns it unchanged.
}

/// A period of local time. The start is included. The end is not included.
pub struct TimeRange { pub start: NaiveDateTime, pub end: NaiveDateTime }
```

| Branch | `range()` | Material |
|---|---|---|
| `Daily` | 09:00 to 12:00 on that day | The activity timeline for this period |
| `Weekly` | Monday 00:00 to the following Monday 00:00 | The full text of the seven daily reports |

- It is written the same way as `LanguageModel`. See "Dispatch" in [LanguageModel](3-language-model.md#9-other-conventions).
- Each branch holds what it needs: the time range, the database, `AiDataPolicy`, and the prompt language. ① builds them.
- Each branch decides how to pick within the limit. For the rules of the daily report, see [Daily report](1-daily-report.md#2-material-the-model-reads).
- To add a kind of report, such as a monthly report, add a branch. The methods do not change.

---

## 2. Tools

This is for the model to query the database by itself. There is only one implementation. So it is a normal struct. For what each tool queries, and what the result looks like, see [Tools](tools/0-overview.md).

```rust
pub struct Tools { /* read-only database connection, AiDataPolicy, prompt language */ }

impl Tools {
    /// All tools shown to the model.
    pub fn specs(&self) -> Vec<ToolSpec>;
    /// Checks the call sent by the model. If it is not valid, the error reason is returned to the model.
    pub fn validate(&self, call: &ToolCall) -> Result<Query, ToolError>;
    /// Executes. Citation numbers start at next_citation. The result is no more than max_chars characters.
    pub async fn execute(&self, query: &Query, next_citation: usize, max_chars: usize)
        -> Result<ToolOutput>;
}

/// A call that has been checked. One branch per tool. For the tool list, see tools/0-overview.md.
pub enum Query { /* … */ }

/// The reason the parameters are not valid. Written for the model to read. The text is in the prompt language.
pub struct ToolError(pub String);

/// The result of one query.
pub struct ToolOutput {
    pub content: String,               // Text put into Message::ToolResult
    pub citations: Vec<Citation>,      // Data citations
}

/// One piece of evidence. The model writes [index] in the answer to cite it.
pub struct Citation {
    pub index: usize,
    pub app: String,
    pub title: String,
    pub started_ts: String,
    pub ended_ts: String,
    pub frame_path: Option<String>,    // The screenshot may already have been removed
}
```

- `ToolSpec` and `ToolCall` are defined in [LanguageModel](3-language-model.md#1-types). One is the tool description given to the model. The other is the call sent by the model. `Query` is a call that has been checked. Progress reports this `Query`.
- Each tool is defined only once: `ToolSpec`, validation, and execution. For how it is translated when given to the model, see [Adapter specification](4-adapters.md#2-chat-completions-protocol-layer-rules). For how it is given to an external app, see [MCP server](1-mcp-server.md).
- The length limit of each result is set from the input budget.
- `Citation` is stored in the chat message. It is also synced. The field names and the JSON format must not be changed.

---

## 3. AiDataPolicy

What AI can see. It is generated once from the settings. Then it is given to `ReportContext` and `Tools`.

- The "Privacy" category (ADR-0019): it is not recorded. So AI cannot see it. If old data was not fully deleted, the tools filter it out too.
- The "Not counted in statistics" category and ignored windows: they are recorded as usual. AI cannot see them by default.
- "Categories for AI analysis" in the settings: the default is every category except "Privacy" and "Not counted in statistics". The user can change it. AI cannot see a category that is not in the list. "Privacy" cannot be added.
- For how the two databases filter the cases above, see [Tools overview 4.1](tools/0-overview.md#41-what-ai-can-see).
- Privacy keywords: AI cannot see a window title that matches. No placeholder is left.
- Screen text (OCR): when writing a report, tools cannot query it by default. Tools can query it only after the user turns on `allowScreenText` in the settings (see [Model selection and configuration](3-settings.md#11-structure-in-the-settings)). Tools in chat can always query it.
- Images are given only to a local model. They can be given to a cloud model only after the user explicitly allows it (see [Screen reader](3-screen-reader.md)).
