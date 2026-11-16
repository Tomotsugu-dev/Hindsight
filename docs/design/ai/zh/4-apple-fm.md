# Apple Intelligence（Apple FM）

> **状态**：草稿，待批准。总体结构见 [AI 后端架构](0-ai-architecture.md)。
>
> **相关**：#247 · #245

Apple Foundation Models（下称 Apple FM）是 macOS 自带的本机语言模型，属于 Apple Intelligence。用户不用下载模型，不用开别的程序，也不用 API Key。

它是 ④ 的一个实现：在进程内调系统框架，不走 HTTP。只在 macOS 上编译，Windows 不受影响。

---

## 1. 需求

**给谁用**：用 Apple 芯片 Mac 的用户。他们想要日报，但不想下载几 GB 的模型，也不想把数据发到云端。

**第一版只做日报。**

- 周报不支持：周报的材料是七天日报的全文，放不进它的上下文（见 [2.5](#25-现在的材料有多大)）。
- 聊天不支持：聊天要调工具，第一版不接工具。

验收标准：

1. 在支持的 Mac 上，日报能选 Apple Intelligence。生成时不起外部进程，不联网。
2. 用不了时，选项灰掉，并写明原因（见[界面](#3-界面)）。
3. 在 macOS 14、15 和 Intel Mac 上，app 照常启动，其他模型照常能用。Windows 不受影响。
4. 一段材料超过上下文长度时，报「这段内容太长」，不报「可能含不安全内容」。
5. 选了它时，周报页写明不支持，不能生成；自动总结跳过周报，不报错。
6. 聊天的模型列表里没有它。
7. 它的输出上限单独定（见[输出上限](#63-输出上限)）。

---

## 2. 查到的事实

每条标了来源：Apple 文档、本机实测、或非官方（论坛、第三方）。本机实测只有一台 Mac：M5，macOS 27.2.1。

### 2.1 哪些 Mac 能用，上下文多长

Apple 文档（[Detecting available Apple Foundation Models](https://developer.apple.com/documentation/foundationmodels/detecting-available-apple-foundation-models)）：

| Mac | macOS 26.0–26.3 | macOS 26.4 起 | macOS 27 |
|---|---|---|---|
| M1、M2、MacBook Neo | AFM 2 Core | AFM 2.1 Core | AFM 3 Core |
| M3 及更新 | AFM 2 Core | AFM 2.1 Core | AFM 3 Core Advanced |
| Intel | 不能用 | 不能用 | 不能用 |

| 模型 | 参数 | 上下文 |
|---|---|---|
| Core | 30 亿 | 4K–8K |
| Core Advanced | 200 亿 | 8K |

- Apple 原话：「Context window size can vary, even within the same model variant and version.」同一个模型、同一个版本，上下文长度也可能不同。
- 另一篇较早的文档（[Managing the context window](https://developer.apple.com/documentation/foundationmodels/managing-the-context-window)）写的是 4096。
- 上下文长度是输入和输出的合计（[`contextSize`](https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel/contextsize) 的文档）。
- 本机实测：8192。
- **所以不写死。** 每次运行时读 `contextSize`。它从 macOS 26.0 起就有。每台 Mac 都按自己的实际值算，不用事先知道各机型的数。
- 机型只影响一件事：上下文是 4K 的 Mac 上，长时段的日报报「太长」会多一些。

### 2.2 中国大陆

- Apple 支持页（[121115](https://support.apple.com/en-us/121115)，2026-11-05）：
  - 在中国大陆购买的设备，目前用不了。
  - 在中国大陆以外购买的设备，如果人在中国大陆、Apple 账户的国家或地区也是中国大陆，也用不了。
- Apple 中国新闻稿（[2026-10-31](https://www.apple.com.cn/newsroom/2026/10/apple-intelligence-features-are-coming-to-users-in-china/)）：iPhone 随 iOS 27.3 在 12 月推出；其他平台「将在获得必要审批后推出」。**现在中国大陆的 Mac 用不了。**
- 这时系统报哪种原因，Apple 没写。论坛里 iPhone 上的报告是 `deviceNotEligible`（「设备不支持」）。非官方，Mac 上没查到。
- 香港、台湾能用。繁体中文从 macOS 26.1 起支持。
- **对我们的影响**：中文用户里中国大陆的用户很多。原因文字要写到中国大陆，不能只写「这台 Mac 不支持」，否则用户会以为是机器不行。发版说明也写明。

### 2.3 语言

- 我们的六种提示词语言：简体中文、英语、日语、葡萄牙语（巴西）、西班牙语从 macOS 26.0 起支持；繁体中文从 26.1 起（[Apple 新闻稿](https://www.apple.com/newsroom/2025/09/new-apple-intelligence-features-are-available-today/)、[支持页](https://support.apple.com/en-us/121115)）。
- 本机实测：`supportsLocale` 对六种语言都返回 true。
- Apple Intelligence 要求设备语言和 Siri 语言设成同一种支持的语言（支持页）。这由系统判断，我们只读结果。
- 运行时用 `supportsLocale` 查提示词语言。不支持就报原因，不发请求。

### 2.4 Token 怎么数

- `tokenCount(for:)` 从 macOS 26.4 起才有。
- 本机实测（Core Advanced）：

  | 内容 | 每个 token 约几个字符 |
  |---|---|
  | 时间线，英文 | 2.1 |
  | 时间线，中文 | 1.8 |
  | 时间线，日文 | 1.9 |
  | 日报提示词，英文、葡语、西语 | 3.7–4.1 |
  | 日报提示词，简体、繁体、日文 | 1.4–1.8 |

- Apple 的说法：英文一个 token 约 3–4 个字符；中日韩文约 1 个字符（[Managing the context window](https://developer.apple.com/documentation/foundationmodels/managing-the-context-window)）。
- macOS 26.0–26.3 上按字符估，取保守的一端：中日韩字符每个算 1 token，其他字符每 3 个算 1 token。

### 2.5 现在的材料有多大

- 提示词（本机实测）：日报 1,071–1,220 token，周报 441–529 token。
- **日报一段**：材料是每小时一行，每个应用最多列 5 个标题。本机用真实数据实测，一段的材料是 400–1,500 token（见 [6.3](#63-输出上限)）。在 4K 上，跨几个小时的长时段可能超；在 8K 上大多放得下。
- **周报**：材料是七天日报的全文。一天的日报常有上千字，七天合起来按中文算有好几千 token。加上提示词和输出，8K 上很紧，4K 上一定超。所以第一版周报不支持。

### 2.6 报错

- 本机实测：输入超过上下文时，permissive 和默认两种 guardrail 都报「超过上下文」（`contextSizeExceeded`），不报「不安全内容」。#247 遇到的「may contain unsafe content」是走 `fm serve` 时的现象，直接调框架时没复现。
- 错误类型随系统变了（Apple 文档）：
  - macOS 26：`LanguageModelSession.GenerationError`。
  - macOS 27：换成 `LanguageModelError` 等新类型，旧的标为弃用。
- 构建机只有 macOS 26 的 SDK（见 [2.8](#28-构建)），代码里写不出 macOS 27 的新类型。所以两种类型都按 `NSError` 的 domain 和 code 认。本机把每种错误构造了一遍，code 就是声明的顺序：

  | 错误 | macOS 26（`…GenerationError`） | macOS 27（`…LanguageModelError`） |
  |---|---|---|
  | 超过上下文 | 0 | 0 |
  | 模型资源不可用 | 1 | — |
  | guardrail 拦下 | 2 | 2 |
  | 拒绝回答 | 8 | 3 |
  | 不支持的语言 | 4 | 7 |
  | 限流 | 6 | 1 |

  domain 分别是 `FoundationModels.LanguageModelSession.GenerationError` 和 `FoundationModels.LanguageModelError`。
- **后台限流**：
  - macOS 26 的文档：只有 app 在后台、并且超过系统限额时才会报（[rateLimited](https://developer.apple.com/documentation/foundationmodels/languagemodelsession/generationerror/ratelimited(_:))）。
  - macOS 上什么算「后台」，官方没写。
  - Apple 工程师在论坛说（非官方）：有界面、在前台的 app 不限流；在后台并且用电池时限流；插着电不该限流。
  - 影响：窗口关着、用电池时，自动日报可能被限流。报 `RateLimited`。要实测。

### 2.7 签名和权限

- 本机实测：ad-hoc 签名、打开 hardened runtime、不带任何 entitlement，调用成功。
- 文档里没有说本机模型要 entitlement。要 entitlement 的是 macOS 27 新加的云端模型（Private Cloud Compute），而且只给 App Store 上的小开发者（[PCC](https://developer.apple.com/private-cloud-compute/)）。我们不用它。
- Apple 工程师在论坛说（非官方）：不上架 App Store 的 app 也能用本机模型。
- 不需要 `fm license`（#247 的实测）。
- Apple 有一份[使用要求](https://developer.apple.com/apple-intelligence/acceptable-use-requirements-for-the-foundation-models-framework/)，禁止的是侵犯他人隐私、按行为给人打分这类事。总结用户自己的活动不在其中。

### 2.8 构建

- 项目已经在编 Swift：依赖 `screencapturekit` 构建时编一个 Swift 包，arm64 和 x86_64 各编一次。
- `swift-rs` 已经在依赖树里（tauri 带进来的，`build` feature 已打开）。用它的 `SwiftLinker` 编译和链接 Swift 包，不新增依赖。
- 最低系统是 macOS 14.0。本机实测：以 14.0 为目标编译时，FoundationModels 自动弱链接（`LC_LOAD_WEAK_DYLIB`），arm64 和 x86_64 都是。老系统上没有这个框架也能启动。Swift 代码包在 `if #available(macOS 26.0, *)` 里。
- SDK 里的 FoundationModels 有 x86_64 版本，所以通用包的 Intel 那一半能编过。运行时系统报「设备不支持」。
- 构建机（[runner-images](https://github.com/actions/runner-images)，2026-11）：

  | workflow | 镜像 | 默认 Xcode | 能用的最新 macOS SDK |
  |---|---|---|---|
  | `ci.yml` | `macos-latest`，即 macOS 26 arm64 | 26.6 | 26.5 |
  | `release.yml` | `macos-15` | 16.4，只有 macOS 15 SDK | 26.2，要手动切到 Xcode 26.3 |

  - `tokenCount` 要 macOS 26.4 的 SDK。所以 `release.yml` 要换到 `macos-26` 镜像。`macos-15` 上手动切 Xcode 也只到 26.2，不够。
  - 两个镜像都没有 macOS 27 的 SDK。所以代码只用 macOS 26 的 API。macOS 27 的新东西（新错误类型、思考档位）第一版不用。
- 没在 macOS 14、15 的真机上验证过启动，只能靠弱链接的静态检查。

### 2.9 实际效果

本机用两天的真实数据跑了日报（Core Advanced，简体中文提示词），对比同一天云端模型写的版本：

- 比云端模型写得长，有一段长一倍。
- 照抄窗口标题，连带标题后缀和标题里的邮箱地址。提示词明说不要抄。
- 有逐小时复述（「00 点到 01 点……01 点到 02 点……」），提示词禁止这样写。
- 有一处把「上午」那段的内容写成「下午」。有一处把收件箱读成了别人发来的邮件。

这些主要是材料和提示词的问题（#245），小模型更容易照着材料抄。M1、M2 上的 Core 只有 30 亿参数，效果没测。

---

## 3. 界面

- **AI 设置的模型列表**：最上面加一张卡。
  - 能用：写「macOS 自带，在这台 Mac 上运行，不联网。暂不支持周报」，有「用于日报」开关，跟云端那张卡一样。
  - 用不了：卡片灰掉，写原因，没有开关。
  - Windows：不显示这张卡。
- **名字**：简体中文界面叫「Apple 智能」，其他语言叫「Apple Intelligence」。这是 Apple 自己在各地的叫法：中国大陆的新闻稿用「Apple 智能」，台湾的新闻稿用「Apple Intelligence」。
  - Apple 的第三方商标规范不允许自造含 Apple 商标的名字，允许指称它的功能（[规范](https://www.apple.com/legal/intellectual-property/guidelinesfor3rdparties.html)）。这里是指称系统功能，不是给我们的功能起名。
  - 不叫「Apple FM」「AFM」：用户不认识。代码里仍叫 `AppleFm`。
- **用不了的原因**：

  | 原因（值见 [5.1](#51-命令)） | 显示 |
  |---|---|
  | `OsTooOld` | 需要 macOS 26 或更新的系统 |
  | `DeviceNotEligible` | 这台 Mac 用不了 Apple Intelligence。需要 M1 或更新的芯片。在中国大陆购买的 Mac，或在中国大陆使用中国大陆 Apple 账户时，目前也用不了 |
  | `NotEnabled` | 请在系统设置里打开 Apple Intelligence |
  | `ModelNotReady` | 系统还在下载模型，请稍后再试 |
  | `LanguageNotSupported` | Apple Intelligence 不支持当前的 AI 提示词语言 |
  | `NotMacos` | 不显示这张卡 |

- **日报页**：选它时，生成前不显示「加载模型中」，因为不用起引擎。
- **周报页**：选它时，显示「Apple Intelligence 暂不支持周报，请在 AI 设置里换一个模型」，生成按钮灰掉。
- **一段生成失败**：沿用现在的段落错误显示。文字走 i18n，错误码见 [5.2](#52--的改动)。

---

## 4. 数据

- `ai.summary_main` 加一个标记值 `__apple_fm__`，跟表示云端的 `__cloud__` 是同一套机制。
- 不加表，不迁移。AI 设置不参与同步。
- 生成的日报照常写进 `ai_summaries`，`model` 列写 `apple-intelligence`（名字待定）。只是换了一个字符串值，格式不变。
- **旧版本**：用户退回旧版本时，旧版会把 `__apple_fm__` 当成模型文件名，报「模型文件不见了」。重新选一个模型就好，数据不丢。

---

## 5. 接口

### 5.1 命令

`apple_intelligence_status`：AI 设置页打开时调用一次。

```rust
pub struct AppleIntelligenceStatus {
    /// 用不了的原因。None 表示能用。
    pub unavailable: Option<UnavailableReason>,
    /// 这台 Mac 的上下文长度（token）。用不了时是 None。
    pub context_size: Option<u32>,
}

/// 模型用不了的原因。定义在 ③，`ModelError::Unavailable` 也用它。
/// 传给前端时写成 snake_case，比如 `device_not_eligible`。
pub enum UnavailableReason {
    NotMacos,
    OsTooOld,
    DeviceNotEligible,
    NotEnabled,
    ModelNotReady,
    LanguageNotSupported,
}
```

| 值 | 什么时候 | 谁判断 |
|---|---|---|
| `NotMacos` | 不是 macOS。前端据此不显示这张卡 | 我们：编译时就知道 |
| `OsTooOld` | macOS 低于 26，系统里没有这个框架 | 我们：查系统版本 |
| `DeviceNotEligible` | 系统报 `deviceNotEligible`。可能是 Intel 芯片，也可能是在中国大陆（见 [2.2](#22-中国大陆)），系统不区分这两种 | 系统 |
| `NotEnabled` | 系统报 `appleIntelligenceNotEnabled`。只有 macOS 26 会出现，macOS 27 起关不掉 | 系统 |
| `ModelNotReady` | 系统报 `modelNotReady`，比如模型还在下载。生成时报「模型资源不可用」也归这里 | 系统 |
| `LanguageNotSupported` | AI 设置里的提示词语言不受支持：查询时 `supportsLocale` 返回 false，或者生成时报「不支持的语言」 | 系统 |

- 先查我们能判断的，再问系统。几种同时成立时，只报第一种。
- 去掉了 `available`：它和「`unavailable` 是不是 None」说的是同一件事，两个字段可能对不上。

### 5.2 ③ 的改动

- `LanguageModel` 加一个分支：`#[cfg(target_os = "macos")] AppleFm(AppleFmModel)`。
- `ModelError` 加三种。错误文字以错误码开头，前端用现有的 `src/lib/aiErrors.ts` 翻译，跟 `LLM_EMPTY_*` 同一套。

  | 错误 | 什么时候 | 错误码 |
  |---|---|---|
  | `Unavailable(UnavailableReason)` | 用不了，原因见 5.1 | `LLM_UNAVAILABLE_` 加上大写的原因，比如 `LLM_UNAVAILABLE_DEVICE_NOT_ELIGIBLE` |
  | `ContextOverflow` | 输入加输出上限超过上下文长度 | `LLM_CONTEXT_OVERFLOW` |
  | `Refused` | 模型拒绝回答，或被 guardrail 拦下 | `LLM_REFUSED` |

- 设计稿里的 `LanguageModelProvider` 第一版不做。「能不能用」由 5.1 的命令直接问适配器。以后做 `Settings` 时再收进去。

### 5.3 Swift 和 Rust 之间

- 都放在 `ai/providers/apple_fm/`：Rust 代码在这里，Swift 包在它下面的 `swift/`。`swift-rs` 把 Swift 的编译产物放在 cargo 的输出目录，源码目录里不多东西。
- Swift 只包一层系统调用，判断都写在 Rust 里，这样 CI 能测到。Swift 导出这些 C 函数，Rust 通过 FFI 调用：

  | 函数 | 做什么 |
  |---|---|
  | `hs_afm_status` | 同步。返回可用性、原因、上下文长度、某种语言支不支持 |
  | `hs_afm_count_tokens` | 异步。数一段文字有多少 token。系统低于 macOS 26.4 时返回「不知道」 |
  | `hs_afm_generate` | 异步。新建一个 session 生成文字，交回文字，或者错误的 domain、code 和说明 |
  | `hs_afm_cancel` | 按任务号取消 |
  | `hs_afm_free` | 释放 Swift 交出来的字符串 |

- 异步的两个函数：起一个 Swift `Task`，马上返回任务号；做完调 Rust 给的回调。
- Rust 这边：回调把结果送进一个 oneshot channel，等它不占线程。future 被丢弃时调 `hs_afm_cancel`。
- 回调一定会被调用一次，取消时也是。这样 Rust 交给 Swift 的指针一定会被收回。

---

## 6. 逻辑

### 6.1 选模型

- 新增 `AiConfig::summary_use_apple_fm()`：`summary_main` 是 `__apple_fm__`，并且是 macOS。
- `effective_summary_main()`、`effective_summary_mmproj()` 遇到 `__apple_fm__` 时，跟遇到 `__cloud__` 一样跳过，回落到 `active_main`。聊天的「自动」沿用这两个函数，所以不会跟到 Apple FM。前端的 `chatRouting.ts` 同样改。
- 聊天的槽位不放 `__apple_fm__`：界面只在日报的槽位显示这张卡。万一存进去，聊天报「选中的本地模型不存在」。`set_step_model` 不另外检查，以后聊天支持 Apple FM 时只改聊天的路由。
- 日报的 runner：选它时跟云端一样，不起引擎。打开模型的地方（`build_step2`）加一个分支。
- 周报的 runner：选它时，开始前就拒绝，报错误码 `AI_WEEKLY_NOT_SUPPORTED`，前端同样用 `aiErrors.ts` 翻译。周报页的提示和灰掉的按钮只是让用户提前看到，拒绝以后端为准。
- 自动总结：选它时跳过周报，不写错误行。判断「没选模型」的地方（`auto_summary.rs`）把它算作选了。

### 6.2 一次生成

1. 查可用性。用不了 → `Unavailable(原因)`，原因按 [5.1](#51-命令) 的顺序判断。
2. 查提示词语言。不支持 → `Unavailable(LanguageNotSupported)`。
3. 数 token：macOS 26.4 起让系统数 instructions 和 prompt；更早的按 2.4 的规则估。
4. 输入加输出上限超过 `contextSize` → `ContextOverflow`，不发请求。
5. 每次新建一个 session：permissive guardrail，instructions 是 system 提示词。不流式。
6. 认错误：

   | 系统报的 | 统一错误 |
   |---|---|
   | 超过上下文 | `ContextOverflow` |
   | guardrail 拦下、拒绝回答 | `Refused` |
   | 限流 | `RateLimited` |
   | 不支持的语言 | `Unavailable(LanguageNotSupported)` |
   | 模型资源不可用 | `Unavailable(ModelNotReady)` |
   | 其他 | `Provider(系统给的说明)` |

- `Request.reasoning`：忽略，`effective_reasoning` 是 `Off`。
- 请求带工具时报 `Provider`。聊天本来选不到它，这只是兜底。
- 用量：第一版不报。

### 6.3 输出上限

- 现在本机和云端用 `summary_max_tokens()`，至少 2048。Apple FM 的上下文短，单独定。
- 上限有两个用处：
  - 发之前留出位置。上下文是输入和输出的合计，预检按「输入 + 上限」算。
  - 防止模型停不下来。
- 本机实测（Core Advanced，两天共 7 段，简体中文提示词）：输入 2,000–2,600 token，输出 142–745 token，每段 4–19 秒。
- 撞上上限时不报错，文字在句子中间被截断（本机实测，上限设 200）。所以上限要明显高于正常长度。
- 定为 1,024 token，比实测最长的多约四成。在 4K 的 Mac 上：4,096 − 提示词约 1,100 − 1,024，留给材料约 1,950 token；实测最长的材料是 1,535 token。
- 没测：M1、M2 上的 Core（30 亿参数）写多长。

---

## 7. 测试

- CI 的构建机是虚拟机，没有 Apple Intelligence，跑不了真模型。CI 只测纯逻辑：错误怎么认、token 怎么估、超长预检、`__apple_fm__` 不影响聊天、周报被拒、自动总结跳过周报。
- 真模型的测试标 `#[ignore]`，在开发者的 Mac 上跑：生成一段、超长报 `ContextOverflow`、取消。
- CI 加一步：检查 FoundationModels 是弱链接。
- 发版前手动做一次：打签名、公证过的通用包，在本机跑一次日报。

---

## 8. 已定和没实测的

已定：

- 周报第一版不支持；日报材料超长只报错。
- 日报一段的输出上限是 1,024 token。
- 名字：`ai_summaries.model` 写 `apple-intelligence`；命令 `apple_intelligence_status`；周报的错误码 `AI_WEEKLY_NOT_SUPPORTED`；Swift 包放在 `ai/providers/apple_fm/swift/`。
- 后台限流：先按 `RateLimited` 报错，实测以后再决定要不要特别处理。

还没实测的：

- 中国大陆的 Mac 上，系统报哪种原因。
- 用 macOS 26 SDK 编出来的程序，在 macOS 27 上收到哪种错误类型。两种都按 domain 和 code 认，所以哪种都能认出来。
- macOS 14、15 上能不能启动。
- 后台限流。
