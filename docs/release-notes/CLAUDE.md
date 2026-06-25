# Release notes 写作规则

## 格式约定

- 每个语言块前加一行隐藏标记 `<!-- zh -->` / `<!-- en -->` / `<!-- ja -->` / `<!-- pt -->`
  （HTML 注释，GitHub 渲染页不可见）。**应用内「检查更新」对话框靠这个标记按用户界面语言
  抽对应块**（见 `src/lib/releaseNotes.ts`）——没有标记的话所有语言会一锅端给用户看。
- 语言顺序：zh → en → ja → pt（界面支持的四种语言都给一份；缺某语言时前端回退英文）
- 每个语言块以 `【x.y.z】` 起头
- 语言块之间空两行
- bullet 用 `-`，单层，不嵌套
- 不用 emoji
- 一条 bullet 一个主题；多件不相关的事不要塞一句话
- 不写 commit hash / PR 号 / 文件路径 / 内部模块名
- 标记码用 2 字母（`zh`/`en`/`ja`/`pt`），前端按界面 locale 前两位匹配（`pt-BR` → `pt`）

## 该写 vs 不该写

| 该写 | 不该写 |
|---|---|
| 用户能看到/感受到的变化 | 内部重构（无观察差异） |
| 性能改善（带数字） | 开发工具（监控脚本、测试 fixture） |
| 修了影响 UX 的 bug | 代码 cleanup / lint / 重命名 |
| 新功能 / 改动了的 workflow | LRU / autoreleasepool / asset 协议这种实现名词 |
| breaking change（migration / 配置格式 / 命令名变） | 依赖升级（除非引入新功能/限制） |

## 风格细节

- **指代用 UI 元素名**：「占比视图」「大类卡」「AI 设置 - 模型」——用户能直接在 app 里定位到
- **不混半中半英**：中文块就全中文，"cosmetic" 写成「显示残留」，"flat" 写成「保持平稳」
- **行为变化要明示**：「现在 X 也会一起做 Y」比「支持 Y」信息密度高
- **breaking change 单独标**：DB migration / 配置格式变 / 命令重命名，bullet 前面加「⚠️ 重大变化：」或「升级前请：」
- **不按 commit 罗列**：把多个相关 commit 归到一条主题 bullet；把无关 commit 拆成多条；commit message 的细节留给 git log

## 数字必须验证

- 写「X MB → Y MB」前必须确认 Y 是**稳态**值，不是 OS 内存压力 / GC 瞬时值
- 不写没跑过的优化幅度（"启动加速 30%" 没 benchmark 就别写）
- 跨平台数字标注观察条件（"macOS 上"），避免被误用到其它系统
- 比较基线要明确（"相比 0.7.4" / "首次安装后立刻测"），避免读者自己脑补

## 同步点（release notes 改完一并更新）

- `src-tauri/tauri.conf.json` 的 `"version"`
- `src-tauri/Cargo.toml` 的 `version`
- `Cargo.lock`：跑一次 `cargo check` 让它自动更新
- GitHub Release tag 必须 `vX.Y.Z` 跟版本号匹配——updater endpoint 走 GitHub Releases 拉 `latest.json`，tag 不对会让所有客户端拉不到更新

## 反面例子（v0.7.5 草稿）

写得太"实现叙事"——把 `图标后端先 resize 到 ≤128px 再编 PNG`、`tokio worker 线程上的 Cocoa 临时对象沉积` 这种实现细节写进了用户面。正确做法是只留**结果**：

> 内存占用从原来 ~360 MB 降到 ~160 MB，长跑也不再持续上涨。

实现叙事留给 commit message / PR 描述 / `docs/internal/` 设计文档，不出现在 release notes。
