# Model selection and configuration

> **Status**: Draft. For the overall structure, see [AI backend architecture](0-ai-architecture.md).

This document covers three things:

- Save the user's choices: which model each task uses, the thinking strength, and the parameters the user changed manually.
- List which models can be chosen, and which ones cannot be used now.
- Before each run, determine the parameters from the model and the task: the thinking strength, the output length, and which method to use. For an item the user changed manually, use the user's value. The context length of a local model is not calculated here. It is left to the engine.

The last two are done by the new module `ai/settings/`. This document is written in three steps: data, interface, and logic.

---

## 1. Data

### 1.1 Structure in the settings

```jsonc
"ai": {
  // The list of cloud connections. Only this needs the user to fill something in, so only this is stored here
  "cloud": [
    {
      "id": "c1",                                 // The id of this connection. picks and advanced below use it to point to this connection
      "name": "DeepSeek",                         // The name the user gave
      "dialect": "deepseek",                      // Which vendor's dialect
      "endpoint": "https://api.deepseek.com/v1",  // Where the request is sent
      "model": "deepseek-chat",                   // Model name
      "apiKey": "…",
      "contextLength": null                       // Context length. The user fills this in only when the provider has not published it and we cannot look it up
    }
  ],

  // Which model each task uses. There are four tasks: daily is the daily report, weekly is the weekly report, chat is chat, and screen is the screen reader
  "picks": {
    "daily":  { "kind": "local", "file": "qwen-4b-q4.gguf" },  // Local model: the file name in the model directory
    "weekly": { "kind": "local", "file": "qwen-4b-q4.gguf" },
    "chat":   { "kind": "cloud", "connection": "c1" },         // Cloud connection: the id of that item in cloud
    "screen": null                                             // Reserved for the screen reader. null = use only OCR
  },

  // Advanced settings: parameters the user changed manually. One item is stored for each "task + model".
  // No such item, or a field is null = use the value calculated by automatic configuration
  "advanced": [
    {
      "task": "daily",                                          // Which task: daily, weekly, chat
      "model": { "kind": "local", "file": "qwen-4b-q4.gguf" },  // Which model. Written the same way as in picks
      "reasoning": "off",        // Thinking strength: one level
      "method": null,            // Which method to use. daily and weekly choose a method for writing a report. chat chooses a method for answering a question
      "maxOutputTokens": null,   // Upper limit of the answer text
      "ctxSize": 32768,          // Context length of the local engine. Only a local model has this
      "parallelSlots": null,     // Number of parallel slots of the local engine. Only a local model has this
      "batchSize": null          // Batch size of the local engine. Only a local model has this
    },
    {
      "task": "chat",
      "model": { "kind": "cloud", "connection": "c1" },
      "reasoning": "high",
      "method": null,
      "maxOutputTokens": null
    }
  ],

  // When writing a report, whether tools can search screen text (OCR). By default, they cannot. Tools in chat can always search it. Read by AiDataPolicy
  "allowScreenText": false
}
```

- Local models are not in the settings. They are the files in the model directory. They are scanned each time.
- Apple FM is not stored either. When it is selected, write `{ "kind": "apple_fm" }` in `picks`.
- Fields that are not related to model selection do not change: `userBrief`, `segments`, `excludedCategories`, `promptLanguage`, `promptOverrides`, `autoSummary`, `autoSummaryTimes`, `modelsPath`.
- The scheduled summary uses `picks.daily`.
- `advanced` is looked up by "task + model". When the model is changed, the items for the other models stay. When you change back, the original values are still there. When a model or a cloud connection is deleted, its items are deleted together with it.

### 1.2 Migration and rollback

- The settings JSON does not take part in sync. It is migrated only on this machine.
- **Migration**: When the settings that are read have no `picks`, generate the new fields from the old fields. The old fields are kept and not deleted.
- **Rollback**: An old version only recognizes the old fields. See [To be decided](#4-to-be-decided) 2.

---

## 2. Interface

### 2.1 Commands for the frontend

All the names are proposals. The form follows the existing commands in the repo: the verb comes first, and the thing it operates on comes after.

| Command | What it does | Returns |
|---|---|---|
| `list_models(task)` | List the models this task can choose | Each item: name, local or cloud, whether it can be used, the reason code when it cannot, the thinking levels that can be selected, and whether it is the one currently selected |
| `set_task_model(task, model)` | Save which model this task uses | — |
| `get_model_parameters(task)` | Read the current value of each parameter of this task | The value of each parameter, and whether it was calculated automatically or set by the user |
| `update_model_parameters(task, patch)` | Change the parameters set by the user, including the thinking strength. A field given as `null` is changed back to automatic. A field that is not given stays as it is | — |
| `create_cloud_connection`, `update_cloud_connection`, `delete_cloud_connection` | Add, change, and delete a cloud connection | — |
| `test_cloud_connection(connection)` | Test a connection. It can be one that has not been saved yet | The result of each step, plus the capabilities that were probed |
| `list_cloud_models(connection)` | Ask the provider for the list of models that can be used | A list of model names |

- A reason code is a fixed string. The text is given by the frontend.
- `get_model_parameters` and `update_model_parameters` read and write the item in `advanced` for "this task + the model that is currently selected".
- Existing commands do not change. The commands that generate the daily report, the weekly report, and chat stay as they are. They read `picks`. The commands that download, import, and delete models stay as they are.

### 2.2 Backend: LanguageModelProvider

One `LanguageModelProvider` is one source of models: the local model directory, a cloud connection, or Apple FM. Each source is asked the same questions, so those questions become one interface, and each source implements it. The selected model is also opened by it.

```rust
/// Functions that every source must implement.
pub trait LanguageModelProviderOps {
    /// Which models are here.
    fn list(&self) -> Vec<ModelInfo>;
    /// Whether this model can be used now. When it cannot, give a reason code.
    fn availability(&self, model: &ModelRef) -> Availability;
    /// Facts about this model: context length, number of parameters, whether it supports tools, whether it can see images, and which thinking levels it has.
    fn describe(&self, model: &ModelRef) -> ModelFacts;
    /// Send one real request to see whether it can be used, and probe its capabilities at the same time.
    async fn probe(&self, model: &ModelRef) -> ProbeReport;
    /// Open the model according to the run configuration. A local model takes an `EngineGuard` in this step.
    async fn open(&self, config: &RunConfig) -> Result<LanguageModel, ModelError>;
}

pub enum LanguageModelProvider {
    Local(LocalProvider),     // The local model directory
    Cloud(CloudProvider),     // A cloud connection
    AppleFm(AppleFmProvider),
}
```

- It is written the same way as `LanguageModel`. See "Dispatch" in [LanguageModel](3-language-model.md#9-other-conventions).
- `open` returns a `LanguageModel`. A local model and a cloud connection return its `ChatCompletions` branch. Apple FM returns the `AppleFm` branch.
- When listing the models that can be chosen, the three sources are put together. What is held is a `Vec<LanguageModelProvider>`.

| Implementation | `list` | `availability` | `describe` | `probe` | `open` |
|---|---|---|---|---|---|
| Local model | Scan the model directory | Is the engine installed? Does the file exist? | Read the GGUF: number of parameters, the context length it was trained with, whether the chat template has tool calls, and whether there is a paired vision projector | Start the engine, and send one minimal request | Get an `EngineGuard`, and return a model in the llama-server dialect |
| Cloud connection | Each item in `cloud` | Are the address and the model name filled in? If login is required, has the user logged in? | The `contextLength` the user filled in, the built-in table of known models, and the result of the last probe | Two steps: can connect and authentication has passed; this model can answer. Then probe tools and the ability to see images | Get the credentials, and return a model in this vendor's dialect |
| Apple FM | One item | Ask the system | Read the context length at runtime | Generate one sentence | No preparation. Return it directly |

`LanguageModelProvider` and `LanguageModel` are a pair:

- `LanguageModelProvider`: which models there are, whether they can be used, and how to open them.
- `LanguageModel`: what you get after it is opened. It is used to call the model.

### 2.3 Module boundaries

- The structure of the settings is added to `AiConfig` (`ai/config.rs`). Reading and writing go through `repo/settings.rs`. A separate set is not written.
- The migration of the old fields is a separate file.
- The new module `ai/settings/` does only two things: list the models one task can choose, and calculate the parameters. It does not read or write the settings. The command passes the `AiConfig` it read to the module. For the module name, see [To be decided](#4-to-be-decided) 6.
- `LanguageModelProvider` is defined in `ai/model/`. The three implementations are in `ai/providers/`. `ai/settings/` uses them only through this interface.
- How to split the files in the module is decided at implementation time.

---

## 3. Logic: automatic configuration

```rust
/// Which model and which parameters one run uses. Everything that can be decided before the model is opened is here.
pub struct RunConfig {
    pub model: ModelRef,
    pub engine: Option<EngineParams>,  // Only a local model has this: requirements for the engine
    pub max_output_tokens: u32,        // Upper limit of the answer text
    pub reasoning: Reasoning,          // Already converted to Default, Off, or a specific level
    pub method: Method,
}

/// Requirements for the local engine.
pub struct EngineParams {
    pub min_ctx_size: u32,             // The context cannot be smaller than this
    pub ctx_size: Option<u32>,         // Context length set by the user. None = let the engine decide
    pub batch_size: Option<u32>,       // Batch size set by the user. None = use the llama.cpp default
    pub parallel_slots: u32,           // Number of parallel slots. If the user did not set it, it is 1
}

pub fn resolve(
    task: Task,            // Which task: daily report, weekly report, chat
    model: &ModelRef,      // The model chosen for this task
    facts: &ModelFacts,    // Facts about this model. The caller reads them first with LanguageModelProvider::describe
    advanced: &Advanced,   // Parameters the user set for "this task + this model". Items that were not set are None
) -> Result<RunConfig, Unavailable>;
```

`resolve` is a pure function. It does not read or write anything. It calculates once before each run. The result is not stored.

The context length is not in `RunConfig`. It is known only after the model is opened. See the end of [3.2](#32-rules).

### 3.1 Two tables

The rules look up two tables. Both are written in the code.

The task defaults table:

| Task | Upper limit of the answer text | Thinking for "Auto" |
|---|---|---|
| One segment of a daily report | Set after real tests | off |
| Weekly report | Set after real tests | off |
| Chat | 1024 | Local: off. Cloud: `Default` |

The method requirements table:

| Method | Kind | Requirements for the model |
|---|---|---|
| Single generation | Write a report | Can produce text |
| Tool loop | Answer a question | Tools; context length not smaller than 8K |
| Summary + tool loop | Write a report | Tools; a longer context, and the number is set after real tests |
| Agent pipeline (reserved) | Not designed yet | Not designed yet |

### 3.2 Rules

Calculate in order.

1. **Can it run**. If the model has none of the capabilities required by the methods this task can use, return `Unavailable`.
2. **Requirements for the engine** (only a local model):
   - We do not calculate the context length or the number of GPU layers. Leave them to llama-server's `--fit`. See [3.4](#34-local-engine).
   - The context is not smaller than 8K.
   - The number of parallel slots is fixed at 1. The batch size uses the llama.cpp default.
3. **Thinking**: The level the user chose falls back according to the levels the model has (see [LanguageModel section 2](3-language-model.md#2-thinking)). When the user chooses "Auto", look up the task defaults table.
4. **Method**: Which method writing a report uses by default will be decided after implementation, once the effect is seen. Answering a question has only the tool loop for now. If the user chose a different one, and the model meets the requirements in the method requirements table, use the one the user chose. If it does not meet them, writing a report falls back to single generation, and the user is told in the progress.
5. **Advanced settings**: Items the user changed replace the values calculated above.

The context length is known only after the model is opened. Read it from the capability description of `LanguageModel`:

| Model | Where the context length comes from |
|---|---|
| Local model | The value the engine reports after it starts |
| Apple FM | Read from the system at runtime |
| Cloud | If the model's facts have it, use it. If not, use a conservative default. See [To be decided](#4-to-be-decided) 3 |

- The input budget is calculated by the method: context length − the output budget that is sent − instructions (see [LanguageModel section 3](3-language-model.md#3-output-budget)).
- When the input budget is too small (proposal: less than 1K), the method reports an error: this model's context is too short, and it cannot do this task.

### 3.3 Examples

| Model and task | What `resolve` calculates |
|---|---|
| Local model, daily report, the user set no parameters | Requirements for the engine: context not smaller than 8K, and the rest is left to the engine. Thinking is off |
| Local model, chat, the user set the context to 16K | Requirements for the engine: the context is 16K. Thinking is off |
| Cloud DeepSeek, chat, thinking "Auto" | No requirements for the engine. Thinking is `Default`. The field is not sent |
| Apple FM, daily report | No requirements for the engine. Thinking is off |

### 3.4 Local engine

`RunConfig.engine` is the requirements this run has for the engine. At one time, llama-server loads only one model and one set of parameters. So when a local model is opened, take these requirements and ask the engine for an `EngineGuard`:

| State of the engine | Result |
|---|---|
| Not running | Start it according to the requirements |
| Running; the model is the same; the context is not smaller than required | Use it directly |
| Running; it does not meet the requirements; no one else is holding a guard | Restart it with the required parameters |
| Running; it does not meet the requirements; someone else is holding a guard | See [To be decided in the architecture](0-ai-architecture.md#4-to-be-decided) 1 |

- `EngineGuard` is placed in the opened `LanguageModel`. When the `LanguageModel` is discarded after use, the guard drops with it and is returned automatically.
- When a guard has not been returned, idle reclaim does not shut down the engine.
- The engine handles only one request at a time. Requests that come at the same time wait in line.

When the engine starts, the context length and the number of GPU layers are left to llama-server's `--fit`. It is on by default. It sets these two from the free memory at that time.

| What we pass | When we pass it | What it does |
|---|---|---|
| `--fit-ctx` | Every time. The value is `min_ctx_size` | The context will not be set smaller than this |
| `--ctx-size` | Passed only when the user set `ctx_size` | The context is opened at the user's value. `--fit` no longer changes it. It only sets the number of GPU layers |
| `-ngl` | Not passed | Let `--fit` set the number of GPU layers |

- After startup is finished, read the actual context length from the engine (`/props`), and put it into the capability description of `LanguageModel`.
- The behavior of `--fit` is written according to the llama.cpp docs. It has not been tested on our machines yet.

---

## 4. To be decided

1. For the context, concurrency, and batch size the user entered manually before: during migration, keep them as values in the advanced settings, and change them to automatic.
2. Does the new version still write the old fields? If it does, the settings still work after rolling back to an old version. If it does not, the models must be chosen again after rolling back.
3. When the context length of a cloud model cannot be looked up, what should the conservative default be? It must be larger than the thinking minimum (12288). Otherwise, no input budget is left.
4. Should the API Key be saved encrypted? The same local key as the Drive token can be used (`sync/local_key.rs`).
5. One cloud connection now carries one model. Usually one provider (address + Key) has more than one model under it. Should this be changed to that? If it is changed, remove `model` from each item in `cloud`, and record both the connection and the model in `picks`.
6. The module name `settings` is the same as `Settings` for the whole app in `repo/settings.rs`. This module only does "list the models that can be chosen" and "calculate the parameters". What should it be renamed to?
7. `--fit` has only a lower limit for the context, and no upper limit. A model with a very long training context is opened to the maximum when there is enough memory, and it uses a lot of memory. Should an upper limit be set, and how should it be set?
8. Should `--fit-target` (the memory margin left for the system) use the default 1024 MiB, or leave more? Decide after real tests.
