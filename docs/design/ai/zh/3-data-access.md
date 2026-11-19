# AI 怎么读数据

> **状态**：草稿。总体结构见 [AI 后端架构](0-ai-architecture.md)。

模型看不到数据库。我们把用户的数据变成文字交给模型，有两种给法：

- `ReportContext` 在调模型之前给材料。给什么由我们的代码定，比如日报给一段时间的活动时间线。日报、周报和 MCP server 用它。
- `Tools` 在模型回答的过程中给查询结果。查什么由模型定，比如模型想看某一小时的完整时间线，就调一次工具。聊天、摘要 + 工具循环和 MCP server 用它。

两种给法都先经过 `AiDataPolicy`。它决定哪些数据不能给模型。

---

## 1. ReportContext

日报、周报里，事先拼好给模型读的文字。

```rust
/// 每种报告的材料都要实现的函数。
pub trait ReportContextOps {
    /// 材料是哪段时间的。
    fn range(&self) -> TimeRange;
    /// 给材料，不超过 max_tokens。用 model 数 token。
    async fn material(&self, max_tokens: u32, model: &LanguageModel) -> Result<String>;
}

/// 一种报告一个分支。
pub enum ReportContext {
    Daily(DailyContext),    // 某天的一段
    Weekly(WeeklyContext),  // 一周
    #[cfg(test)]
    Fixed(String),          // 测试用，原样返回
}

/// 一段本地时间，含开始，不含结束。
pub struct TimeRange { pub start: NaiveDateTime, pub end: NaiveDateTime }
```

| 分支 | `range()` | 材料 |
|---|---|---|
| `Daily` | 当天 09:00 到 12:00 | 这段时间的活动时间线 |
| `Weekly` | 周一 00:00 到下周一 00:00 | 七天的日报全文 |

- 写法和 `LanguageModel` 一样，见 [LanguageModel](3-language-model.md#9-其他约定) 的「分发」。
- 每个分支自己带着要用的东西：时间范围、数据库、`AiDataPolicy`、提示词语言。由 ① 建好。
- 上限之内怎么挑，每个分支自己定。日报的规则见[日报](1-daily-report.md#2-模型读到的材料)。
- 加一种报告，比如月报，就加一个分支。方法不用改。

---

## 2. Tools

模型自己查库用。只有一个实现，所以是普通的结构体。每个工具查什么、结果长什么样，见[工具](tools/0-overview.md)。

```rust
pub struct Tools { /* 只读的数据库连接、AiDataPolicy、提示词语言 */ }

impl Tools {
    /// 给模型看的全部工具。
    pub fn specs(&self) -> Vec<ToolSpec>;
    /// 检查模型发来的调用。不合法时，错误原因还给模型。
    pub fn validate(&self, call: &ToolCall) -> Result<Query, ToolError>;
    /// 执行。引用编号从 next_citation 开始，结果不超过 max_chars 个字。
    pub async fn execute(&self, query: &Query, next_citation: usize, max_chars: usize)
        -> Result<ToolOutput>;
}

/// 检查过的调用。一个工具一个分支，工具清单见 tools/0-overview.md。
pub enum Query { /* … */ }

/// 参数不合法的原因。写给模型看，按提示词语言写。
pub struct ToolError(pub String);

/// 一次查询的结果。
pub struct ToolOutput {
    pub content: String,               // 放进 Message::ToolResult 的文字
    pub citations: Vec<Citation>,      // 数据引用
}

/// 一条证据。模型在答案里写 [index] 引用它。
pub struct Citation {
    pub index: usize,
    pub app: String,
    pub title: String,
    pub started_ts: String,
    pub ended_ts: String,
    pub frame_path: Option<String>,    // 截图可能已经被清理
}
```

- `ToolSpec` 和 `ToolCall` 定义在 [LanguageModel](3-language-model.md#1-类型)：一个是给模型的工具说明，一个是模型发来的调用。`Query` 是检查过的调用，进度里报的就是它。
- 每个工具只写一份定义：`ToolSpec`、校验、执行。交给模型时怎么翻译，见[适配规格](4-adapters.md#2-chat-completions协议层的规则)；交给外部应用，见 [MCP server](1-mcp-server.md)。
- 每条结果的长度上限，按输入预算定。
- `Citation` 存在聊天消息里，还参与同步。字段名和 JSON 格式不能改。

---

## 3. AiDataPolicy

AI 能看到什么。按设置生成一次，交给 `ReportContext` 和 `Tools`。

- 「隐私」分类（ADR-0019）：不记录，本来就看不到。旧数据没删干净时，工具也过滤掉。
- 「不计入统计」分类、已忽略的窗口：照常记录，AI 默认看不到。
- 设置里「AI 分析的分类」：默认是「隐私」「不计入统计」以外的全部分类，用户可以改。不在里面的分类，AI 看不到。「隐私」加不进去。
- 上面几种，两个库怎么过滤，见[工具总览 4.1](tools/0-overview.md#41-可见范围)。
- 隐私关键词：命中的窗口标题，AI 看不到，也不留占位。
- 屏幕文字（OCR）：写报告时，工具默认查不到。用户打开设置里的 `allowScreenText` 以后才能查（见[模型选择和配置](3-settings.md#11-设置里的结构)）。聊天的工具一直能查。
- 图片只给本机的模型。用户明确允许以后，才能给云端的模型（见[读屏](3-screen-reader.md)）。
