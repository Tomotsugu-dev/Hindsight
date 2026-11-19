# Screen reader

> **Status**: Only the interface is specified. It is not implemented. For the overall structure, see [AI backend architecture](0-ai-architecture.md).
>
> **Related**: #16

The screen reader gets text from a screenshot. #16 wants to use a cloud model to read screenshots. This time we only leave the interface ready. We do not make a cloud model see images.

---

## 1. Interface

```rust
pub trait ScreenReader {
    async fn read(&self, frame: &Path) -> Result<FrameText>;
}

pub enum FrameText {
    Lines(Vec<String>),     // Text lines from OCR
    Description(String),    // The model's description of the screenshot
}
```

- The memory database calls it when digesting a screenshot (`Recognizer` in `memory/digest.rs`).
- **OCR**: This is the only implementation in the first version. On macOS it uses Apple Vision. On other platforms it uses Paddle.
- **Model that sees images**: Any `LanguageModel` with `vision = true` can be a screen reader. This is not done this time.

`picks.screen` in the settings decides which one to use. `null` means use only OCR (see [Model selection and configuration](3-settings.md#11-structure-in-the-settings)).

---

## 2. Threshold

To send a screenshot to a model with `on_device = false`, the user must turn on a new setting. The default is off. This rule is in `AiDataPolicy` (see [How AI reads data](3-data-access.md#3-aidatapolicy)).

This setting will change the principle "screenshots never go to the cloud". So write an ADR at implementation time.

---

## 3. To be decided at implementation time

1. Which screenshots are worth sending to the model? Sending every screenshot will cost a lot.
2. Where is the model's description stored? Should it go into the full-text index together with the OCR text?
