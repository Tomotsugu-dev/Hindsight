# 模型选择和配置

> **状态**：草稿。总体结构见 [AI 后端架构](0-ai-architecture.md)。

本文讲三件事：

- 存用户的选择：每个任务用哪个模型、思考强度、手动改过的参数。
- 列出有哪些模型可选，哪些现在用不了。
- 每次运行前，按模型和任务定参数：思考强度、输出长度、用哪个方法。用户手动改过的项，用用户的值。本机模型的上下文长度不在这里算，交给引擎。

后两件由新模块 `ai/settings/` 做。本文按数据、接口、逻辑三步写。

---

## 1. 数据

### 1.1 设置里的结构

```jsonc
"ai": {
  // 云端连接的列表。只有它需要用户填东西，所以只有它存在这里
  "cloud": [
    {
      "id": "c1",                                 // 这个连接的编号。下面的 picks 和 advanced 用它指这个连接
      "name": "DeepSeek",                         // 用户起的名字
      "dialect": "deepseek",                      // 哪一家的方言
      "endpoint": "https://api.deepseek.com/v1",  // 请求发到哪
      "model": "deepseek-chat",                   // 模型名
      "apiKey": "…",
      "contextLength": null                       // 上下文长度。服务商没公布、我们也查不到时，用户才要填
    }
  ],

  // 每个任务用哪个模型。任务有四个：daily 日报、weekly 周报、chat 聊天、screen 读屏
  "picks": {
    "daily":  { "kind": "local", "file": "qwen-4b-q4.gguf" },  // 本机模型：模型目录里的文件名
    "weekly": { "kind": "local", "file": "qwen-4b-q4.gguf" },
    "chat":   { "kind": "cloud", "connection": "c1" },         // 云端连接：cloud 里那一项的 id
    "screen": null                                             // 读屏预留。null = 只用 OCR
  },

  // 高级设置：用户手动改过的参数。一个「任务 + 模型」记一项。
  // 没有这一项，或者字段是 null = 用自动配置算出来的值
  "advanced": [
    {
      "task": "daily",                                          // 哪个任务：daily、weekly、chat
      "model": { "kind": "local", "file": "qwen-4b-q4.gguf" },  // 哪个模型。写法和 picks 里一样
      "reasoning": "off",        // 思考强度：一个档位
      "method": null,            // 用哪个方法。daily、weekly 选写报告的方法，chat 选回答问题的方法
      "maxOutputTokens": null,   // 正文上限
      "ctxSize": 32768,          // 本机引擎的上下文长度。只有本机模型有
      "parallelSlots": null,     // 本机引擎的并发路数。只有本机模型有
      "batchSize": null          // 本机引擎的批大小。只有本机模型有
    },
    {
      "task": "chat",
      "model": { "kind": "cloud", "connection": "c1" },
      "reasoning": "high",
      "method": null,
      "maxOutputTokens": null
    }
  ],

  // 写报告时，工具能不能查屏幕文字（OCR）。默认不能。聊天的工具一直能查。由 AiDataPolicy 读
  "allowScreenText": false
}
```

- 本机模型不存在设置里。它们就是模型目录里的文件，每次现扫。
- Apple FM 也不存。选它时 `picks` 里写 `{ "kind": "apple_fm" }`。
- 和模型选择无关的字段不变：`userBrief`、`segments`、`excludedCategories`、`promptLanguage`、`promptOverrides`、`autoSummary`、`autoSummaryTimes`、`modelsPath`。
- 定时总结用 `picks.daily`。
- `advanced` 按「任务 + 模型」找。换模型时，别的模型的项留着，换回来原来的值还在。删掉模型或云端连接时，它的项一起删。

### 1.2 迁移和回退

- 设置 JSON 不参与同步，只在本机迁移。
- **迁移**：读到设置里没有 `picks` 时，用旧字段生成新字段。旧字段留着不删。
- **回退**：旧版本只认旧字段。见[待决定](#4-待决定) 2。

---

## 2. 接口

### 2.1 给前端的命令

名字都是提议。写法照仓库里现有的命令：动词在前，后面是它操作的东西。

| 命令 | 做什么 | 返回 |
|---|---|---|
| `list_models(task)` | 列出这个任务可选的模型 | 每一项：名字、本机或云端、能不能用、不能用的原因码、可选的思考档位、是不是当前选中的 |
| `set_task_model(task, model)` | 存这个任务用哪个模型 | — |
| `get_model_parameters(task)` | 读这个任务的参数现在各是多少 | 每个参数的值，以及它是自动算的还是用户设的 |
| `update_model_parameters(task, patch)` | 改用户设的参数，包括思考强度。字段给 `null` 就是改回自动，没给的字段不动 | — |
| `create_cloud_connection`、`update_cloud_connection`、`delete_cloud_connection` | 添加、修改、删除云端连接 | — |
| `test_cloud_connection(connection)` | 测试一个连接，可以是还没保存的 | 每一步的结果，加探测到的能力 |
| `list_cloud_models(connection)` | 向服务商要可用的模型列表 | 模型名的列表 |

- 原因码是固定的字符串，文字由前端给。
- `get_model_parameters` 和 `update_model_parameters` 读写的是 `advanced` 里「这个任务 + 当前选中的模型」那一项。
- 已有的命令不变：生成日报、周报、聊天的命令照旧，它们读 `picks`；下载、导入、删除模型的命令照旧。

### 2.2 后端：LanguageModelProvider

一个 `LanguageModelProvider` 是模型的一个出处：本机的模型目录、云端连接、Apple FM。对每个出处要问的是同样几个问题，所以抽成一个接口，各实现一遍。选中的模型也由它打开。

```rust
/// 每个出处都要实现的函数。
pub trait LanguageModelProviderOps {
    /// 这里有哪些模型。
    fn list(&self) -> Vec<ModelInfo>;
    /// 这个模型现在能不能用。不能用时给出原因码。
    fn availability(&self, model: &ModelRef) -> Availability;
    /// 这个模型的事实：上下文长度、参数量、支不支持工具、看图、有哪些思考档位。
    fn describe(&self, model: &ModelRef) -> ModelFacts;
    /// 真发一次请求，看它能不能用，顺便探测能力。
    async fn probe(&self, model: &ModelRef) -> ProbeReport;
    /// 按运行配置打开模型。本机模型在这一步拿 `EngineGuard`。
    async fn open(&self, config: &RunConfig) -> Result<LanguageModel, ModelError>;
}

pub enum LanguageModelProvider {
    Local(LocalProvider),     // 本机的模型目录
    Cloud(CloudProvider),     // 云端连接
    AppleFm(AppleFmProvider),
}
```

- 写法和 `LanguageModel` 一样，见 [LanguageModel](3-language-model.md#9-其他约定) 的「分发」。
- `open` 交出 `LanguageModel`。本机模型和云端连接交出它的 `ChatCompletions` 分支，Apple FM 交出 `AppleFm` 分支。
- 列出可选的模型时，三个出处的要放在一起，拿的是 `Vec<LanguageModelProvider>`。

| 实现 | `list` | `availability` | `describe` | `probe` | `open` |
|---|---|---|---|---|---|
| 本机模型 | 扫模型目录 | 引擎装了吗；文件在吗 | 读 GGUF：参数量、训练时的上下文长度、对话模板里有没有工具调用、有没有配对的视觉投影 | 启动引擎，发一次最小请求 | 拿 `EngineGuard`，交出 llama-server 方言的模型 |
| 云端连接 | `cloud` 里的每一项 | 地址和模型名填了吗；要登录的登录了吗 | 用户填的 `contextLength`，内置的已知模型表，上次探测的结果 | 两步：连得上、认证过；这个模型能回答。再探测工具和看图 | 取凭证，交出这一家方言的模型 |
| Apple FM | 一项 | 问系统 | 运行时读上下文长度 | 生成一句话 | 不用准备，直接交出 |

`LanguageModelProvider` 和 `LanguageModel` 是一对：

- `LanguageModelProvider`：有哪些模型、能不能用、怎么打开。
- `LanguageModel`：打开以后拿到的东西，用来调模型。

### 2.3 模块的边界

- 设置的结构加在 `AiConfig`（`ai/config.rs`）里。读写走 `repo/settings.rs`，不另写一套。
- 旧字段的迁移单独一个文件。
- 新模块 `ai/settings/` 只做两件事：列出一个任务可选的模型，算参数。它不读写设置，命令把读出来的 `AiConfig` 交给它。模块名见[待决定](#4-待决定) 6。
- `LanguageModelProvider` 定义在 `ai/model/`，三个实现在 `ai/providers/`。`ai/settings/` 只通过这个接口用它们。
- 模块里的文件怎么分，实现时定。

---

## 3. 逻辑：自动配置

```rust
/// 一次运行用什么模型、什么参数。打开模型之前就能定的，都在这里。
pub struct RunConfig {
    pub model: ModelRef,
    pub engine: Option<EngineParams>,  // 只有本机模型有：对引擎的要求
    pub max_output_tokens: u32,        // 正文上限
    pub reasoning: Reasoning,          // 已经翻成 Default、Off 或具体档位
    pub method: Method,
}

/// 对本机引擎的要求。
pub struct EngineParams {
    pub min_ctx_size: u32,             // 上下文不能小于它
    pub ctx_size: Option<u32>,         // 用户设的上下文长度。None = 交给引擎定
    pub batch_size: Option<u32>,       // 用户设的批大小。None = 用 llama.cpp 的默认值
    pub parallel_slots: u32,           // 并发路数。用户没设就是 1
}

pub fn resolve(
    task: Task,            // 哪个任务：日报、周报、聊天
    model: &ModelRef,      // 这个任务选的模型
    facts: &ModelFacts,    // 这个模型的事实。调用方先用 LanguageModelProvider::describe 读好
    advanced: &Advanced,   // 用户给「这个任务 + 这个模型」设的参数。没设的项是 None
) -> Result<RunConfig, Unavailable>;
```

`resolve` 是一个纯函数，不读写任何东西。它每次运行前算一次，结果不存。

上下文长度不在 `RunConfig` 里。它要等模型打开了才知道，见 [3.2](#32-规则) 末尾。

### 3.1 两张表

规则要查两张表，都写在代码里。

任务默认值表：

| 任务 | 正文上限 | 「自动」的思考 |
|---|---|---|
| 日报的一段 | 实测后定 | 关 |
| 周报 | 实测后定 | 关 |
| 聊天 | 1024 | 本机：关。云端：`Default` |

方法需求表：

| 方法 | 哪一类 | 对模型的要求 |
|---|---|---|
| 单次生成 | 写报告 | 能出文字 |
| 工具循环 | 回答问题 | 工具；上下文长度不小于 8K |
| 摘要 + 工具循环 | 写报告 | 工具；上下文较长，数值实测后定 |
| Agent 流水线（预留） | 还没设计 | 还没设计 |

### 3.2 规则

按顺序算。

1. **能不能跑**。这个任务能用的方法，要的能力模型都没有，返回 `Unavailable`。
2. **对引擎的要求**（只有本机模型）：
   - 上下文长度和 GPU 层数我们不算，交给 llama-server 的 `--fit`，见 [3.4](#34-本机引擎)。
   - 上下文不小于 8K。
   - 并发路数固定为 1。批大小用 llama.cpp 的默认值。
3. **思考**：用户选的档位，按模型有的档位降级（见 [LanguageModel 第 2 节](3-language-model.md#2-思考)）。用户选「自动」时，查任务默认值表。
4. **方法**：写报告默认用哪个，实现以后看效果再定。回答问题现在只有工具循环。用户选了别的，而且模型满足方法需求表里的要求，就用用户选的；不满足，写报告退回单次生成，并在进度里告诉用户。
5. **高级设置**：用户改过的项，替换上面算出来的值。

上下文长度打开模型以后才知道，从 `LanguageModel` 的能力描述里读：

| 模型 | 上下文长度从哪来 |
|---|---|
| 本机模型 | 引擎启动后报的值 |
| Apple FM | 运行时向系统读 |
| 云端 | 模型的事实里有就用；没有就用保守的默认值，见[待决定](#4-待决定) 3 |

- 输入预算由方法算：上下文长度 − 发出去的输出预算 − instructions（见 [LanguageModel 第 3 节](3-language-model.md#3-输出预算)）。
- 输入预算太小时（提议：不到 1K），方法报错：这个模型的上下文太短，做不了这个任务。

### 3.3 例子

| 模型和任务 | `resolve` 算出来的 |
|---|---|
| 本机模型，日报，用户没设参数 | 对引擎的要求：上下文不小于 8K，其余交给引擎。思考关 |
| 本机模型，聊天，用户把上下文设成 16K | 对引擎的要求：上下文 16K。思考关 |
| 云端 DeepSeek，聊天，思考「自动」 | 没有对引擎的要求。思考是 `Default`，不发字段 |
| Apple FM，日报 | 没有对引擎的要求。思考关 |

### 3.4 本机引擎

`RunConfig.engine` 是这次运行对引擎的要求。llama-server 同一时刻只加载一个模型、一组参数，所以本机模型打开时，要拿着这份要求向引擎要一个 `EngineGuard`：

| 引擎的状态 | 结果 |
|---|---|
| 没在跑 | 按要求启动 |
| 在跑；模型相同；上下文不小于要求 | 直接用 |
| 在跑；不满足要求；没有别人拿着 guard | 重启成要求的参数 |
| 在跑；不满足要求；有别人拿着 guard | 见[架构的待决定](0-ai-architecture.md#4-待决定) 1 |

- `EngineGuard` 放在打开的 `LanguageModel` 里。`LanguageModel` 用完被丢弃时，guard 跟着 drop，自动归还。
- 还有 guard 没归还时，空闲回收不关引擎。
- 引擎一次只处理一个请求。同时来的请求排队。

启动引擎时，上下文长度和 GPU 层数交给 llama-server 的 `--fit`。它默认开着，按当时空闲的内存定这两项。

| 我们传什么 | 什么时候传 | 作用 |
|---|---|---|
| `--fit-ctx` | 每次。值是 `min_ctx_size` | 上下文不会被定得比它小 |
| `--ctx-size` | 用户设了 `ctx_size` 才传 | 上下文照用户的值开，`--fit` 不再改它，只定 GPU 层数 |
| `-ngl` | 不传 | GPU 层数让 `--fit` 定 |

- 启动完成后，向引擎读实际的上下文长度（`/props`），放进 `LanguageModel` 的能力描述。
- `--fit` 的行为按 llama.cpp 的文档写，还没有在我们的机器上实测。

---

## 4. 待决定

1. 用户以前手填的上下文、并发、批大小，迁移时保留成高级设置里的值，改成自动。
2. 新版本还写不写旧字段？写的话，退回旧版本后设置还能用；不写的话，退回后要重新选模型。
3. 云端模型的上下文长度查不到时，保守的默认值取多少？它必须比思考底线（12288）大，否则留不出输入预算。
4. API Key 要不要加密保存？可以用 Drive token 那套本机密钥（`sync/local_key.rs`）。
5. 现在一个云端连接带一个模型。通常一个 provider（地址 + Key）下有多个模型。要不要改成这样？改的话，`cloud` 里的每一项去掉 `model`，`picks` 里记连接和模型两样。
6. 模块名 `settings` 和 `repo/settings.rs` 里整个应用的 `Settings` 撞名。这个模块只做「列出可选的模型」和「算参数」，改叫什么？
7. `--fit` 只有上下文的下限，没有上限。训练上下文很长的模型，内存够时会被开到最大，占很多内存。要不要设上限，怎么设？
8. `--fit-target`（留给系统的内存余量）用默认的 1024 MiB，还是多留一些？实测后定。
