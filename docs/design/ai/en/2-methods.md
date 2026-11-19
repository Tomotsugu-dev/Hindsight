# Methods

> **Status**: Draft. For the overall structure, see [AI backend architecture](0-ai-architecture.md).

A method is "how to use a model to finish one job". The same job can have several methods. Changing the method does not require changing the feature entry point. It does not require changing the model implementation.

① calls the method. `Settings` chooses which method to use, based on the model's capabilities (see [Model selection and configuration](3-settings.md#3-logic-automatic-configuration)).

---

## 1. Two kinds of methods

Methods are split into two kinds by "what job they do". Methods in the same kind have the same input and the same output. They can be swapped.

| Kind | Input | Output | Which methods |
|---|---|---|---|
| Write a report | instructions, and a `ReportContext` that can provide material | Text, citations, usage | Single generation; summary + tool loop; Agent pipeline (reserved) |
| Answer a question | The question, the whole conversation (with the compact cache) | Text, citations, usage, the compact result of this turn | Tool loop |

```rust
// Write a report
pub struct ReportInput {
    pub instructions: String,              // Writing requirements
    pub context: ReportContext,            // Which period to write, and the material, are both requested from here
}
pub struct ReportOutput {
    pub text: String,
    pub citations: Vec<Citation>,          // Citations of the result
    pub usage: Usage,                      // Tokens consumed
}

// Answer a question
pub struct Question {
    pub text: String,
    pub history: Vec<HistoryTurn>,         // Complete, from the root to before this question
}
pub struct Answer {
    pub text: String,
    pub citations: Vec<Citation>,
    pub usage: Usage,
    pub new_summary: Option<NewSummary>,   // Present only if this turn did compact
}

pub enum HistoryTurn {
    User(String),
    Assistant {
        text: String,
        citations: Vec<Citation>,
        summary: Option<String>,           // Cache: the compact result from the root to this entry
    },
}

pub struct NewSummary {
    pub last_index: usize,                 // Which history entry compact reaches, including this one. Must be an answer
    pub text: String,                      // The compact result
}
```

- For `ReportContext`, see [ReportContext](3-data-access.md#1-reportcontext). For `Citation`, see [Tools](3-data-access.md#2-tools). For `Usage`, see [LanguageModel](3-language-model.md#1-types).
- One kind of report corresponds to one branch of `ReportContext`. ① chooses the branch. ① supplies the instructions for that kind of report. Methods do not distinguish reports.
- The time range is taken with `context.range()`. When tools are used, this tells the model the dates.
- The output holds only the result. How many times the model was called, and how it finished, go through the progress callback (see [section 2](#2-what-to-pass-when-calling-a-method)).
- The method decides how much to put in the context. The method first calculates the input budget: the context length of `model` − the output budget that is sent − instructions. Material is requested from `ReportContext` up to this limit. ① gives the whole conversation.
- ① attaches the method's compact result to the answer at `last_index`. If that entry is not an answer, ① does not attach it. Only the branches under this answer use it.
- The compact result is only a cache. When the original text fits, the method uses the original text. When it does not fit, the method uses the latest compact result plus the original text after it. If it is lost, it can be compacted again.

When answering a question, earlier context is used like this:

- **Question rewrite**: When there is earlier context, call the model once first. Rewrite the new question into a question that can be understood on its own. For example, the previous sentence asks "How long did writing code take last week?". This sentence asks "What about Chrome?". It is rewritten as "How long did using Chrome take last week?". If the rewrite succeeds, the earlier context is not included when answering. The output limit of this step is 160 tokens. The method sets this limit itself. It is not in `RunConfig`.
- **History cleaning**: Include the latest few turns of earlier context only when the rewrite fails. Before including them, remove the citation numbers in the old answers. This turn's evidence is numbered again. So copying the old numbers would point to the wrong records. Content that is too long is cut short.

---

## 2. What to pass when calling a method

Every method that writes a report has a function like this. A method that answers a question is the same. The function is named `answer_question`. The input and output are changed to `Question` and `Answer`.

```rust
async fn generate_report(
    input: ReportInput,           // Input from section 1
    model: &LanguageModel,        // The model ① has already opened
    config: &RunConfig,           // Upper limit of the answer text, thinking level
    tools: &Tools,                // Only methods that use tools call this
    lang: PromptLanguage,         // Which language the method's own text uses
    progress: &dyn Fn(Progress),  // The method calls this to report progress
) -> Result<ReportOutput>;
```

- ① opens the model, then passes it in. The segments of a daily report share one opening. The local engine is prepared only once. When testing, pass a fake model.
- **Cancel**: Drop the method's future.
- **Progress**: The method reports which step it has reached. It reports which tool it called and the parameters. It reports how it finished. ① turns this into frontend events. The progress includes the user's search terms and the time range. The log records this only at debug level.

---

## 3. Guardrails for the tool loop

These rules belong to the tool loop. Whoever uses the tool loop carries them:

- **Step limit**: The most times the model can be called in one answer.
- **Repeated calls**: The same tool with the same parameters runs only once. The second time, tell the model to change the parameters.
- **Parameter check**: When the parameters are not valid, return the error reason to the model as the tool result. Let the model change them itself.
- **Fallback answer**: When the model fails several times in a row, or the steps are used up, answer directly with the evidence already collected. If that still does not work, give one honest sentence that explains the failure. Do not invent anything.
- **Citation binding**: Tool results carry numbers. If the answer cites a number that does not exist, remove it.

Citation numbers increase globally within one run. The caller gives the starting number. So when one method uses the tool loop several times, they can share one counter.

---

## 4. Summary + tool loop

One method for "write a report". The details are not decided yet. See [To be decided](#6-to-be-decided) 2–4.

1. The model reads the material that `ReportContext` gives.
2. The model decides which windows to look at in detail. It uses tools to query them.
3. The material and the details that were found are given to the model together. The model writes this segment.

- It uses the tool loop. For the guardrails, see [section 3](#3-guardrails-for-the-tool-loop).
- The details found are more detailed than the material: titles removed because of the budget, the start and end time of each title, and screen text. (Screen text cannot be searched by default. It can be searched only after the user turns it on in settings.)
- The model needs to be able to call tools. The context also needs to be longer. How long it needs to be is set after real tests.

---

## 5. When the context is too long

This refactor only blocks with the limit. It does not compact. For how to do it, see [To be decided](#6-to-be-decided) 1.

---

## 6. To be decided

1. The material for the weekly report is the full text of seven days of daily reports. When it does not fit in the limit, which part does the weekly report's `ReportContext` shrink first?
