# 插件市场

总体见 [插件后端架构](plugins-architecture.md)。

**本文旨在设计**：插件从哪里来。包括三件事：

- 插件作者怎么上架；
- Hindsight 去哪找插件、从哪下载；
- 怎么保证下载到的文件没被改过、插件 id 不重复；
- 下载次数从哪来，市场页怎么排序、筛选。

**上架**

```mermaid
flowchart LR
  A["插件作者"] -->|"发版<br/>上传 zip"| REL[("作者自己的仓库<br/>Release")]
  A -->|"提 PR<br/>加一条记录"| IDX[("官方仓库<br/>plugins.json")]
  O["Hindsight 维护者"] -->|"审 PR、合并"| IDX
```

**安装**

```mermaid
sequenceDiagram
  participant H as Hindsight<br/>插件市场页
  participant I as 官方仓库 main<br/>plugins.json
  participant S as 官方仓库 stats 分支<br/>downloads.json
  participant R as 作者自己的仓库<br/>Release
  Note over H: 用户打开市场页
  par 同时读
    H->>I: 读列表
  and
    H->>S: 读下载次数
  end
  Note over H: 用户点安装
  H->>R: 按列表里的地址<br/>下载 zip
```

- 官方仓库只放列表文件 `plugins.json` 和插件图标 `icons/<id>.png`，不放插件的代码。zip 放在作者自己仓库的 Release 里。
- 市场页的图标从官方仓库读，不从作者的服务器读：否则一打开市场页，就会请求每个作者的服务器。装好以后用包里的 `icon.png`，审 PR 时确认两张是同一张图。
- 官方仓库单独建一个，不放进 Hindsight 主仓库：上架的 PR 不和应用代码混在一起，也不用跑主仓库的 CI。
- 插件 id 不重复，由审 PR 保证。
- 市场页只在用户打开时读一次列表，不在后台定时去拉。
- 读不到列表（断网、10 秒超时、格式不认识）时，「可下载」那一栏显示原因，「已安装」和本地 zip 两栏照常显示。

### 列表文件的格式

外面一层是格式版本；`plugins` 里每个插件一条记录，`versions` 里每个版本一条：

```json
{
  "format_version": 2,
  "plugins": [
    {
      "id": "hindsight-focus",
      "name": "专注",
      "author": "someone",
      "description": {
        "zh-CN": "离今天的目标还差多少，切到娱乐时提醒",
        "en": "How far you are from today's goal. Reminds you when you switch to entertainment."
      },
      "repo": "someone/hindsight-focus",
      "versions": [
        {
          "version": "0.2.0",
          "url": "https://github.com/someone/hindsight-focus/releases/download/0.2.0/hindsight-focus-0.2.0.zip",
          "sha256": "…",
          "permissions": ["stats.read", "stats.today", "focus.changed", "focus.category", "notifications"],
          "storage_mb": 10
        },
        {
          "version": "0.1.0",
          "url": "https://github.com/someone/hindsight-focus/releases/download/0.1.0/hindsight-focus-0.1.0.zip",
          "sha256": "…",
          "permissions": ["stats.read", "stats.today", "focus.changed", "focus.category"]
        }
      ]
    }
  ]
}
```

- `format_version` 是这个文件结构的版本。Hindsight 先读它，不认识就报错，不解析后面的插件。
- 某个插件的条目解析不了（少了字段、类型不对），Hindsight 跳过这个插件，其余的照常显示。0.10.2 及更早的版本会整份报错、打不开市场，所以合并 PR 前仍要确认格式对。
- `description` 是各种语言的简介，键是界面的语言代码：`zh-CN`、`zh-TW`、`en`、`ja`、`pt-BR`、`es`。市场页按界面语言显示；没有这种语言就用 `en`，`en` 也没有就用语言代码排序后的第一个。
- 同一个 `id` 的插件有新版本时，提 PR 在 `versions` 里加一条，旧版本可以选择保留。
- 每个版本自带 `permissions` 和 `storage_mb`，因为不同版本申请的可能不一样。`storage_mb` 是给插件自己的库申请的磁盘容量（MB），不用库的插件不写，上面的 0.1.0 就没写。市场页在下载前就把这个版本的权限和磁盘容量给用户看，用户同意的传给安装，存进 plugins 表（见 [① 插件安装](install.md)）。

### 下载次数

hindsight-plugins 每天统计一次各插件的下载次数，写进 `stats` 分支的 `downloads.json`。怎么统计见那个仓库的 [count_downloads.py](https://github.com/Tomotsugu-dev/hindsight-plugins/blob/main/scripts/count_downloads.py)。

```json
{
  "format_version": 1,
  "downloads": {
    "hindsight-focus": 10,
    "hindsight-reminder": 6
  }
}
```

- 不让 Hindsight 自己查 GitHub API：不登录的请求，每个 IP 每小时只有 60 次。
- Hindsight 读不到这个文件时，市场照常打开，不显示下载次数，按 plugins.json 的顺序排。

## 市场页的三栏

| 栏 | 内容 | 数据从哪来 |
|---|---|---|
| 已安装 | 装了的插件。有新版本时显示「更新到 x」，旁边的下拉可以选线上的任一版本 | plugins 表，加上线上同 id 的版本、简介和下载次数 |
| 可下载 | 线上有、还没装的插件，按下载次数从多到少排 | plugins.json 和 downloads.json |
| 本地 zip | `plugin-zips/` 里的包（见 [① 插件安装](install.md)） | 文件夹 |

- 「可下载」按 id 分：装了的插件不管线上有没有新版本，都不在「可下载」栏中。
- 「已安装」软件如果可更新，则显示在「已安装」里。
- 排序：下载次数多的在前；一样多或没有数的，保持 plugins.json 的顺序。
- 筛选：「下载最多」显示全部，是默认的；「官方插件」只显示官方的。
- 搜索：找名称、作者、仓库名和界面语言的简介，不分大小写，只在本机筛。
- 卡片和已安装的插件多一行：下载次数和仓库链接。链接只显示仓库名，作者已经在上面了。线上列表里没有的插件，不显示这一行和简介。
