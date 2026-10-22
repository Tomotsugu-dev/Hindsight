# timer：倒计时

![timer](../../images/island/timer.svg)

左边是收起时，右边是展开时，都是过了四分之一的时候。环随时间变短，旁边是剩下的时间：一小时以上是 `h:mm:ss`，不到一小时是 `m:ss`。

```json
{ "type": "timer", "start": "2026-10-21T10:00:00+09:00", "end": "2026-10-21T10:25:00+09:00", "color": "#fb923c" }
```

| 字段 | 必须有吗 | 说明 |
|---|---|---|
| `start` | 是 | 开始的时间，RFC 3339，和 `focus.changed` 的 `at` 一样 |
| `end` | 是 | 结束的时间，格式同上 |
| `pausedAt` | 否 | 暂停的时间，格式同上。有它时，停在那一刻的剩余时间 |
| `color` | 否 | 环的颜色，`#rrggbb`，不传是白色 |

- 计时器由 Hindsight 倒数，插件不用每秒调一次。暂停时插件再调一次 `island.show`，计时器带上 `pausedAt`。
- 到 0 以后停在 0，等插件换内容。
- 环的两头之间留一条缝：剩的时间是满的，也看得出是个环。

## 允许的放置位置

| 位置 | 样子 |
|---|---|
| `closed.left`、`closed.right` | 14 像素的环，小字 |
| `open.left`、`open.right` | 20 像素的环，大字 |

## 取值限制

不合就报错：

- `start` 要早于 `end`。
- `pausedAt` 要在 `start` 和 `end` 之间。
