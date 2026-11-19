# Search parameters

> **Status**: Draft, waiting for approval.
>
> **Scope**: Parameters shared by [search titles](activity.md) and [search screen text](ocr.md). The code is in `kinds/search/mod.rs`. For the path of parameters from the model to here, see [Overview 3.4](../0-overview.md#34-one-call).

---

## 1. Parameters given to the model

| Parameter | JSON name | Required | Description |
|---|---|---|---|
| Keywords | `keywords` | Yes | 1 to 3. Each is at most 64 characters. It is a match only if every one appears |
| Date range | `date_from`, `date_to` | No | `YYYY-MM-DD`. If only the start is given, the range goes up to today. If only the end is given, the range starts from the earliest. If neither is given, search all |
| App | `apps` | No | Search only these apps |
| Category | `categories` | No | Search only apps in these categories |
| How many to list | `limit` | No | Default 8. At most 20 |

The length limit is not a parameter given to the model. The caller passes it in through `execute`. See [Overview 3.3](../0-overview.md#33-the-tool-trait-and-the-registry).

---

## 2. Checked parameters

```rust
// kinds/search/mod.rs
/// Checked parameters.
pub struct SearchParams {
    keywords: Vec<String>,                  // 1 to 3, each at most 64 characters
    range: Option<(NaiveDate, NaiveDate)>,  // if not given, search all
    apps: Vec<String>,                      // search only these apps; if not given, search all apps
    categories: Vec<String>,                // search only these categories; if not given, search all categories
    limit: usize,                           // how many to list, default 8
}
```

The two tools each wrap one layer: `ActivityQuery(SearchParams)` and `OcrQuery(SearchParams)`. `Tool` is implemented for a type. The same type can implement it only once.

When the parameters are not valid, write the reason for the model, so it can change them and call again:

- Keywords: empty after the leading and trailing whitespace is removed, more than 3, or one of them is longer than 64 characters.
- Dates: the format is wrong, the start is later than the end, or a date is later than today.
- How many to list: less than 1, or over the upper limit.
