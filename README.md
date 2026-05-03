# Hindsight

> 一个本地运行的电脑使用记录工具，帮你回看「今天到底把时间花在了哪里」。

Hindsight 在后台静默地记录你在电脑上看到了什么、用了什么应用、停留了多久；之后用图表、应用排行、定时截图三种角度让你"事后回看"。

数据全部存在本地，默认不上传任何东西。

---

## 功能

- **焦点窗口采集** — 每秒检测当前聚焦的窗口；切换应用 / 窗口标题就立刻开一个新会话，记录到 SQLite
- **整窗截图** — 焦点切换瞬间抓一张当前窗口的截图，等比缩放到 1280×720 上限，JPEG 80 存盘；按日期分子目录
- **今日 / 本周 / 本月总览** — 24 小时堆叠柱状图、每日/每周柱状图、应用排行、分类排行；左右切换日期带平滑过渡
- **应用分类** — 用户自定义分类（颜色 + 图标 + 应用绑定），未分类应用会单独列出方便逐个指派
- **工作时段过滤** — 设置上下班时段，时段外不采集
- **设备视图** — 多设备区分（云同步铺垫）
- **AI 总结 / 设置**（占位） — 后续接入本地大模型分析

## 技术栈

| 层 | 实现 |
|---|---|
| 桌面壳 | [Tauri 2](https://tauri.app/) |
| 前端 | React 19 + TypeScript + Vite + react-router-dom + CSS Modules |
| 后端 | Rust 2021 + tokio + tokio-rusqlite |
| 数据 | SQLite（带版本化迁移） |
| 抓窗 / 截图 | [xcap](https://crates.io/crates/xcap) |
| 应用图标提取 | Win32 (`SHGetFileInfo` + GDI) / macOS (`icns` + `plist`) |
| 图标素材 | [lucide-react](https://lucide.dev/) |

支持平台：**Windows** / **macOS**（Linux 未测试，理论可跑）。

## 本地开发

```sh
# 装依赖（Node 端）
npm install

# 启动 dev（vite + tauri 一起拉起，热重载）
npm run tauri dev

# 打 release 包
npm run tauri build
```

需要本地有 Rust toolchain（`rustup` 默认稳定版）+ Tauri 2 prerequisites（Windows 需要 WebView2 Runtime；macOS 需要 Xcode CLT）。

## 数据存放位置

- 默认数据根：`%APPDATA%/Hindsight/`（Windows）/ `~/Library/Application Support/Hindsight/`（macOS）
- 主数据库：`<data_root>/hindsight.sqlite`
- 截图根：`<data_root>/screenshots/YYYY-MM-DD/HHMMSS_mmm.jpg`
- 数据根可以在「设置 → 常规 → 数据保存路径」修改（重启生效，旧数据需手动迁移）

## 状态

仍在迭代中，UI / Schema 都可能再调整。Phase 1 / 2 的本地能力（采集 + 分类 + 总览）已经能用，Phase 3 的云同步与 AI 总结刚开始铺路。

## License

MIT
