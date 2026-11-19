# 搜索的参数

> **状态**：草稿，待批准。
>
> **范围**：[查数据](activity.md)和[查 OCR](ocr.md) 共用的参数，代码在 `kinds/search/mod.rs`。参数从模型传到这里的路线，见[总览 3.4](../0-overview.md#34-一次调用)。

---

## 1. 给模型的参数

| 参数 | JSON 名 | 必填 | 说明 |
|---|---|---|---|
| 关键词 | `keywords` | 是 | 1 到 3 个，每个最多 64 字。每个都出现才算命中 |
| 日期范围 | `date_from`、`date_to` | 否 | `YYYY-MM-DD`。只给开始，算到今天；只给结束，从最早算起；都不给就搜全部 |
| 应用 | `apps` | 否 | 只搜这些应用 |
| 分类 | `categories` | 否 | 只搜这些分类里的应用 |
| 列几个 | `limit` | 否 | 默认 8，最多 20 |

长度上限不是给模型的参数，由调用方经 `execute` 传进来，见[总览 3.3](../0-overview.md#33-tool-trait-和登记处)。

---

## 2. 检查过的参数

```rust
// kinds/search/mod.rs
/// 检查过的参数。
pub struct SearchParams {
    keywords: Vec<String>,                  // 1 到 3 个，每个最多 64 字
    range: Option<(NaiveDate, NaiveDate)>,  // 不给就搜全部
    apps: Vec<String>,                      // 只搜这些应用；不给就搜全部应用
    categories: Vec<String>,                // 只搜这些分类；不给就搜全部分类
    limit: usize,                           // 列几个，默认 8
}
```

两个工具各包一层：`ActivityQuery(SearchParams)`、`OcrQuery(SearchParams)`。`Tool` 按类型实现，同一个类型只能实现一次。

不合法时，把原因写给模型，让它改了再调：

- 关键词：去掉首尾空白后是空的、超过 3 个、某个超过 64 字。
- 日期：格式不对、开始晚于结束、晚于今天。
- 列几个：小于 1，或者超过上限。
