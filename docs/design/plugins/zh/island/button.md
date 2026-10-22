# button：按钮

![button](../../images/island/button.svg)

只能放在展开时。左边的按钮只有图标，右边的有图标也有字。

```json
{ "type": "button", "id": "pause", "icon": "Pause" }
```

```json
{ "type": "button", "id": "skip", "icon": "SkipForward", "text": "跳过" }
```

| 字段 | 必须有吗 | 说明 |
|---|---|---|
| `id` | 是 | 插件自己起的名字，被点时原样交回给插件 |
| `icon` | 是 | 图标的名字，和 [`icon`](icon.md) 的 `name` 一样 |
| `text` | 否 | 按钮上的字 |

- 按钮被点时不打开页面，插件的后台脚本收到 `island.clicked`，内容是 `{ "button": "pause" }`。不在后台运行的插件放了按钮，`island.show` 报错：它收不到点击。

## 允许的放置位置

- `open.left`、`open.right`。收起时放不下。

## 长度限制

- `id` 最长 64 字节。
- `text` 最多 20 宽，宽度怎么算见 [岛](../island.md) 的「各处容器空间」。不能是空的，也不能有换行这类控制字符。
