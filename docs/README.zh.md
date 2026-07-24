
<p align="center">
  <img src="../src/assets/logo.png" alt="Hindsight" width="200">
</p>

<h1 align="center">Hindsight</h1>

<p align="center">
  <i>你的电脑日记 — 它替你记得每一天。</i>
</p>

<p align="center">
  <a href="README.zh.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="../README.md">English</a> · <a href="README.ja.md">日本語</a> · <a href="README.pt.md">Português</a>
</p>

<p align="center">
  <a href="https://github.com/Tomotsugu-dev/Hindsight/releases">
    <img alt="GitHub Release" src="https://img.shields.io/github/v/release/Tomotsugu-dev/Hindsight?color=blue&logo=github">
  </a>
  <img alt="Windows" src="https://img.shields.io/badge/Windows-0078D4?logo=microsoftwindows&logoColor=white">
  <img alt="macOS" src="https://img.shields.io/badge/macOS-000000?logo=apple&logoColor=white">
  <a href="../LICENSE">
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
  </a>
</p>

---

## 为什么是 Hindsight

你是不是也常常凌晨合上电脑，感觉自己「忙了一整天」，却说不上今天到底做成了什么？前阵子我想找个时间追踪工具来解决这个问题，市面上挑了一圈都没用下去：

- **[ActivityWatch](https://github.com/ActivityWatch/activitywatch)** — 开源、隐私优先，功能上挑不出毛病。但实话讲，它的界面没什么吸引力，装完打开看过一次，之后就再没点开过。
- **[WorkReview](https://github.com/wm94i/Work-Review) 这类工具** — 我想要两件事同时满足：一是能跨设备汇总，二是像 iPhone「屏幕使用时间」那样按小时缩放的时间轴，让我直接看到「下午 3 点我在干嘛」。桌面端没有一款做到让我满意。
- **[Toggl](https://toggl.com) / [RescueTime](https://www.rescuetime.com) / 各种付费 SaaS** — 这些本质上是给团队和 HR 算「计费工时」用的：仪表盘信息密集，流程围着项目打标签转，数据还要传到对方的云。我要的是「自己跟自己复盘」，方向完全对不上。

为了解决以上这些问题，Hindsight 应运而生。

## 界面预览

<p align="center">
  <video src="https://github.com/user-attachments/assets/e6349610-a742-4ba2-abca-412f00b1673c" controls muted autoplay loop playsinline width="800"></video>
</p>
<p align="center">
  <sub><i><b>软件预览</b> · 1 分钟看清 Hindsight 的核心交互</i></sub>
</p>

<p align="center">
  <img src="./intro_zh/imgs/today.png" alt="今日总览" width="800"><br/>
  <sub><i><b>今日总览</b> · 24 小时分时段堆叠图 × 应用排行榜，一眼看清今天的时间去向，工作学习节奏</i></sub>
</p>

<p align="center">
  <img src="./intro_zh/imgs/monthly.png" alt="月统计" width="800"><br/>
  <sub><i><b>月统计</b> · 每日柱状 × 月度排行，看清长期工作节奏</i></sub>
</p>

<p align="center">
  <img src="./intro_zh/imgs/ai_summary.png" alt="AI 总结" width="800"><br/>
  <sub><i><b>AI 自动写日报</b> · 本地 LLM 按时段看截图，输出段落式总结；截图始终本地</i></sub>
</p>

## 主要功能

- 📊 **看清时间花在哪** — 后台自动记录，分时段柱状图 + 应用排行；按周 / 月汇总；可自定义分类（"工作 / 娱乐 / 学习"）
- 🤖 **AI 自动写日报**（新）— 本地 LLM 读你的截图，按时段写出段落式总结
- ☁️ **多设备汇总** — 可选 Google Drive 同步活动数据，多台电脑一处查看（截图始终留在本地）
- 🔒 **完全本地、隐私优先** — 数据默认仅存本机；只在设定的工作时段记录；自动跳过登录 / 密码页截图

## 快速开始

从 [Releases](https://github.com/Tomotsugu-dev/Hindsight/releases) 下载对应平台的安装包并安装。

### Windows

下载 `hindsight_x.y.z_x64-setup.exe`，双击安装即可。

> ⚠️ **首次运行会弹出「Windows 已保护你的电脑」** — 安装包尚未购买 EV 代码签名证书，会被 SmartScreen 拦下。点击「更多信息」→「仍要运行」即可继续安装。

### MacOS

下载 `hindsight_x.y.z_universal.dmg`（Apple Silicon + Intel 通用二进制），双击挂载后将 Hindsight 拖入「应用程序」即可正常打开——应用已接入 Apple 开发者证书签名 + 公证，不会再触发 Gatekeeper 警告。

> 所有活动数据 / 截图默认仅存本地。如果开启 Google Drive 同步，只会上传活动元数据，**不会上传截图**。

## 技术栈

| 类别 | 技术 |
|---|---|
| 桌面框架 | [Tauri 2](https://tauri.app/) |
| 前端 | React 19 · TypeScript · Vite |
| 后端 | Rust · Tokio · SQLite · reqwest |
| AI 推理 | [llama.cpp](https://github.com/ggml-org/llama.cpp) · Qwen2.5-VL / Qwen3-VL · OpenAI 兼容 API |
| 同步 | Google Drive API |

## License

<p align="center">
  本项目基于 <a href="../LICENSE"><b>MIT License</b></a> 开源，欢迎自由使用、修改与分发。<br/>
  <sub>© 2026 Hindsight contributors</sub>
</p>
