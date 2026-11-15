# 方法

> **状态**：草稿。总体结构见 [AI 后端架构](0-ai-architecture.md)。

方法是「怎么用模型完成一件事」。同一件事可以有几种方法，换方法不用改功能入口，也不用改模型实现。

方法由 ① 调用。用哪个方法，由 `Settings` 按模型的能力选（见[模型选择和配置](3-settings.md#3-逻辑自动配置)）。

---

## 1. 两类方法

方法按「做什么事」分成两类。同一类里的方法输入输出相同，可以互换。

| 类 | 输入 | 输出 | 有哪些方法 |
|---|---|---|---|
| 写报告 | instructions、能要到材料的 `ReportContext` | 文字、引用、用量 | 单次生成；摘要 + 工具循环；Agent 流水线（预留） |
| 回答问题 | 问题、整段对话（带着 compact 的缓存） | 文字、引用、用量、这一轮 compact 的结果 | 工具循环 |

```rust
// 写报告
pub struct ReportInput {
    pub instructions: String,              // 写作要求
    pub context: ReportContext,            // 写哪段时间、材料，都从这里要
}
pub struct ReportOutput {
    pub text: String,
    pub citations: Vec<Citation>,          // 结果的引用
    pub usage: Usage,                      // 消耗 Tokens
}

// 回答问题
pub struct Question {
    pub text: String,
    pub history: Vec<HistoryTurn>,         // 从根到这次提问之前，完整的
}
pub struct Answer {
    pub text: String,
    pub citations: Vec<Citation>,
    pub usage: Usage,
    pub new_summary: Option<NewSummary>,   // 这一轮 compact 了才有
}

pub enum HistoryTurn {
    User(String),
    Assistant {
        text: String,
        citations: Vec<Citation>,
        summary: Option<String>,           // 缓存：根到这一条 compact 的结果
    },
}

pub struct NewSummary {
    pub last_index: usize,                 // compact 到 history 的哪一条，含这一条。必须是回答
    pub text: String,                      // compact 的结果
}
```

- `ReportContext` 见 [ReportContext](3-data-access.md#1-reportcontext)，`Citation` 见 [Tools](3-data-access.md#2-tools)，`Usage` 见 [LanguageModel](3-language-model.md#1-类型)。
- 一种报告对应 `ReportContext` 的一个分支。① 选分支，配上这种报告的 instructions。方法不区分报告。
- 时间范围用 `context.range()` 拿。用工具时，要靠它告诉模型日期。
- 输出只放结果。调了几次模型、怎么收尾，走进度回调（见[第 2 节](#2-调用方法时传什么)）。
- 上下文里放多少由方法定。方法先算输入预算：`model` 的上下文长度 − 发出去的输出预算 − instructions。材料按这个上限向 `ReportContext` 要，对话由 ① 整段给。
- 方法 compact 的结果，① 挂在 `last_index` 那条回答上，不是回答就不挂。只有这条回答下面的分支用得到它。
- compact 的结果只是缓存。装得下时方法用原文，装不下才用最近一次 compact 的结果加后面的原文。丢了可以重新 compact。

回答问题时，上文这样用：

- **问题改写**：有上文时，先调一次模型，把新问题改写成单看也懂的问题。比如上一句问「上周写代码用了多久」，这一句问「那 Chrome 呢」，改写成「上周用 Chrome 用了多久」。改写成功，作答时不带上文。这一步的输出上限是 160 token，由方法自己定，不在 `RunConfig` 里。
- **历史消毒**：改写失败，才带上最近几轮上文。带之前去掉旧回答里的引用编号，因为这一轮的证据会重新编号，照抄旧编号会指错记录。过长的内容截短。

---

## 2. 调用方法时传什么

每个写报告的方法都有这样一个函数。回答问题的方法也一样，函数叫 `answer_question`，输入输出换成 `Question`、`Answer`。

```rust
async fn generate_report(
    input: ReportInput,           // 第 1 节的输入
    model: &LanguageModel,        // ① 已经打开好的模型
    config: &RunConfig,           // 正文上限、思考档位
    tools: &Tools,                // 只有用工具的方法会调
    lang: PromptLanguage,         // 方法自己的文字用哪种语言
    progress: &dyn Fn(Progress),  // 方法调它报进度
) -> Result<ReportOutput>;
```

- 模型由 ① 打开以后传进来。日报的几段共用一次打开，本机引擎只准备一次。测试时传一个假模型。
- **取消**：丢弃方法的 future。
- **进度**：方法报到了哪一步、调了哪个工具和参数、怎么收尾的。① 把它转成前端事件。进度里有用户的搜索词和时间段，日志只记 debug 级。

---

## 3. 工具循环的护栏

这些规则属于工具循环，谁用工具循环谁就带着它们：

- **步数上限**：一次回答最多调几次模型。
- **重复调用**：同样的工具、同样的参数只执行一次。第二次告诉模型换个参数。
- **参数校验**：参数不合法时，把错误原因作为工具结果还给模型，让它自己改。
- **降级作答**：模型连续失败或步数用完时，用已有的证据直接作答。还不行就给一句诚实的失败说明，不编造。
- **引用绑定**：工具结果带编号。答案里引用了不存在的编号，就把它去掉。

引用编号在一次运行里全局递增。起点由调用方给，所以一个方法里用几次工具循环时，可以共用一个计数。

---

## 4. 摘要 + 工具循环

「写报告」的一种方法。细节还没定，见[待决定](#6-待决定) 2–4。

1. 模型读 `ReportContext` 给的材料。
2. 模型决定要细看哪些窗口，用工具去查。
3. 材料和查到的细节一起给模型，写出这一段。

- 用工具循环，护栏见[第 3 节](#3-工具循环的护栏)。
- 查到的细节比材料细：按预算被挑掉的标题、每个标题的起止时间、屏幕文字。（屏幕文字默认查不到，用户在设置里打开以后才能查。）
- 需要模型会调工具，上下文也要长一些。要多长，实测后再定。

---

## 5. 上下文太长时

这次重构只靠上限挡住，不做 compact。怎么做见[待决定](#6-待决定) 1。

---

## 6. 待决定

1. 周报的材料是七天的日报全文。放不进上限时，周报的 `ReportContext` 先缩哪部分？
