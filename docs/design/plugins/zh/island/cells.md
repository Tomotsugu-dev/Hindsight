# cells：格子

![cells](../../images/island/cells.svg)

只能放在灵动岛展开时。图里每行 10 格，一共 14 格，第 8 格是当前那格。

```json
{
  "type": "cells",
  "colors": ["#a78bfa", "#a78bfa", "#a78bfa", "#a78bfa", "#a78bfa", "#f87171", "#a78bfa", "#a78bfa", null, null, null, null, null, null],
  "columns": 10,
  "current": 7
}
```

| 字段 | 必须有吗 | 说明 |
|---|---|---|
| `colors` | 是 | 每格一个颜色，`#rrggbb`；空格子是 `null` |
| `columns` | 是 | 每行几格 |
| `current` | 否 | 当前那格的下标，从 0 数。这一格有白框 |

- 格子最大 10 像素，不跟着面板变宽：三行要放得进面板。
- `open.right` 放不下时，左边一列让出位置，格子变小。

## 允许的放置位置

- `open.left`、`open.right`。收起时放不下。

## 数量限制

不符合就报错：

- `columns` 是 1 到 20。
- 最多 3 行，所以 `colors` 最长为 60 。`colors` 不能是空的。
- `current` 要落在格子里。
