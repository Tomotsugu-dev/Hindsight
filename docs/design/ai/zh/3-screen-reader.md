# 读屏

> **状态**：只定接口，不实现。总体结构见 [AI 后端架构](0-ai-architecture.md)。
>
> **相关**：#16

读屏就是从一张截图里拿到文字。#16 想用云端模型读截图。这次只把接口留好，不做云端看图。

---

## 1. 接口

```rust
pub trait ScreenReader {
    async fn read(&self, frame: &Path) -> Result<FrameText>;
}

pub enum FrameText {
    Lines(Vec<String>),     // OCR 的文字行
    Description(String),    // 模型对截图的描述
}
```

- 记忆库消化截图时调用它（`memory/digest.rs` 的 `Recognizer`）。
- **OCR**：第一版唯一的实现。macOS 上用 Apple Vision，其他平台用 Paddle。
- **看图模型**：任何 `vision = true` 的 `LanguageModel` 都可以做读屏。这次不做。

用哪个，由设置里的 `picks.screen` 决定。`null` 就是只用 OCR（见[模型选择和配置](3-settings.md#11-设置里的结构)）。

---

## 2. 门槛

截图要发给 `on_device = false` 的模型，必须用户打开一个新设置，默认关。这条规则放在 `AiDataPolicy` 里（见 [AI 怎么读数据](3-data-access.md#3-aidatapolicy)）。

这个设置会改变「截图永远不上云」的原则，所以实现时要写 ADR。

---

## 3. 实现时再定

1. 哪些截图值得发给模型？每张都发，费用会很高。
2. 模型的描述存在哪里？要不要和 OCR 文字一起进全文索引？
