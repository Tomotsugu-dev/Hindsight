# ring：进度环

![ring](../../images/island/ring.svg)

左边是收起时，右边是展开时，`value` 都是 0.75。

```json
{ "type": "ring", "value": 0.75, "text": "75%", "color": "#34d399" }
```

| 字段 | 必须有吗 | 说明 |
|---|---|---|
| `value` | 是 | 0 到 1 的进度 |
| `text` | 否 | 收起时在环的右边，展开时在环的中间 |
| `color` | 否 | 环的颜色，`#rrggbb`，不传是白色 |

- `value` 是 1 时画满一圈：1 是做完了。[`timer`](timer.md) 的环两头留缝，`ring` 不留。

## 允许的放置位置

| 位置 | 样子 |
|---|---|
| `closed.left`、`closed.right` | 14 像素的环 |
| `open.left`、`open.right` | 52 像素的环 |

## 长度和取值限制

- `text` 最多 6 宽，宽度怎么算见 [岛](../island.md) 的「各处容器空间」。
- `text` 不能是空的，也不能有换行这类控制字符。
- `value` 不在 0 到 1 之间就报错。
