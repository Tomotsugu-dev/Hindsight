# 岛

总体见 [插件后端架构](plugins-architecture.md)。

**本文旨在设计**：插件在菜单栏正中间的岛上放一点内容，比如专注插件的剩余时间。鼠标移上去岛会展开，点一下打开插件的页面。包括三件事：

- 插件交给岛的内容是什么格式，谁来画；
- 每处能放多少；
- 几个插件同时要用岛时显示谁。

方法见 [③ 插件请求数据](api.md) 的 `island.show`、`island.hide`。

## 岛上的几块地方

### 收起时

![收起的岛](../images/island-closed.svg)

上面是有刘海的屏幕，下面是没有刘海的屏幕。

| 位置 | 能放什么 |
|---|---|
| `closed.left` | 一个 [`icon`](island/icon.md)、[`timer`](island/timer.md)、[`ring`](island/ring.md) 或 [`text`](island/text.md) |
| `closed.right` | 同上 |

- 左右两边至少放一边。
- 刘海、胶囊的宽度和展开动画由 Hindsight 处理，插件不用管屏幕有没有刘海。两边的宽度都跟着内容，只放一个图标时是圆：有刘海时，岛从刘海两边各伸出这么宽，最多 90，没放东西的一边不伸出去；没有刘海时，两边各是一块胶囊。
- `text` 只能是小字，不带 `sub`：收起时只有菜单栏那么高。

### 展开时

![展开的岛](../images/island-open.svg)

| 位置 | 能放什么 |
|---|---|
| `closed.left` | 留在原处，和收起时一样 |
| `open.topRight` | 一个小字的 `text`。有刘海时在刘海右边，没有刘海时紧跟在 `closed.left` 后面。有它时 `closed.right` 不显示，没它时 `closed.right` 留在原处 |
| `open.left` | 最多 3 个组件，从上往下排，靠左 |
| `open.right` | 最多 2 个组件，排成一行，靠右 |
| `open.bottomRight` | 一个小字的 `text`，在 `open.right` 那一行下面，靠右 |

- 展开的岛宽高都跟着内容，最宽 520。
- `open` 可以不传：不传时鼠标移上去不展开，点一下照样打开插件的页面。

## 内容的格式

例子：专注插件开始一段 25 分钟的专注。收起时左边是分类图标，右边是倒计时；展开时最上面一条右边是「工作中 · Code」，左边一列是今天做了多久和一排格子，右边一行是倒计时和暂停按钮，按钮下面是分心次数。

```json
{
  "closed": {
    "left": { "type": "icon", "name": "Code", "color": "#a78bfa" },
    "right": { "type": "timer", "start": "2026-10-21T10:00:00+09:00", "end": "2026-10-21T10:25:00+09:00", "color": "#fb923c" }
  },
  "open": {
    "topRight": { "type": "text", "text": [{ "text": "工作中", "bold": true }, { "text": " · Code", "dim": true }] },
    "left": [
      { "type": "text", "text": [{ "text": "2:40", "bold": true }], "sub": "/ 6:00", "size": "large" },
      { "type": "cells", "colors": ["#a78bfa", "#a78bfa", "#f87171", null, null], "current": 2, "columns": 15 }
    ],
    "right": [
      { "type": "timer", "start": "2026-10-21T10:00:00+09:00", "end": "2026-10-21T10:25:00+09:00", "color": "#fb923c" },
      { "type": "button", "id": "pause", "icon": "Pause" }
    ],
    "bottomRight": { "type": "text", "text": ["分心 ", { "text": "3", "color": "#f87171", "bold": true }, " 次"] }
  }
}
```

每个组件一篇，字段、能放在哪、上限都在里面：

| 组件 | 是什么 | 收起时能放吗 |
|---|---|---|
| [`icon`](island/icon.md) | 图标 | 能 |
| [`text`](island/text.md) | 一行字 | 能，只能是小字 |
| [`timer`](island/timer.md) | 倒计时 | 能 |
| [`ring`](island/ring.md) | 进度环 | 能 |
| [`cells`](island/cells.md) | 格子 | 不能 |
| [`button`](island/button.md) | 按钮 | 不能 |

岛的规矩：

- 内容由 Hindsight 画，插件不能放自己的 HTML：能放 HTML 就能画出假的系统弹窗。
- 点岛不会让 Hindsight 变成前台 app：不然专注插件会把这次点击记成切到了 Hindsight。
- 不认识的组件跳过，不报错：新插件在旧版 Hindsight 上只少显示一块。
- 组件格式是给插件作者的约定：可以加组件、加字段，已有字段的意思不改。
- 颜色只能是 `#rrggbb`，太暗的由 Hindsight 调亮：插件可以直接用分类的颜色。

## 各处容器空间

文字按显示宽度算：全角字显示宽度为 2，其余每个字符算为1。每个组件的上限写在它自己那篇里。

- 上限按岛最宽的时候来定：每处都放满，岛也不会比 520 宽。
- 超过上限就报错：作者开发时就能发现。

## 几个插件都要用岛

- 后调用 `island.show` 的插件占着岛，它撤下以后，前一个插件回到岛上。
- 插件被停用、卸载，或者后台停了，它的内容就撤掉。
- 点岛打开的是占着岛的插件的页面，在单独的窗口里打开。
