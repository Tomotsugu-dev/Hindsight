
<p align="center">
  <img src="./src/assets/logo.png" alt="Hindsight" width="200">
</p>

<h1 align="center">Hindsight</h1>

<p align="center">
  <i>本地运行的电脑活动记录工具 — 追踪你一天里使用过的应用（可选云同步）</i>
</p>

<p align="center">
  <a href="README.md">中文</a> · <a href="README.en.md">English</a> · <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  <a href="https://github.com/Tomotsugu-dev/Hindsight/releases">
    <img alt="GitHub Release" src="https://img.shields.io/github/v/release/Tomotsugu-dev/Hindsight?color=blue&logo=github">
  </a>
  <img alt="Windows" src="https://img.shields.io/badge/Windows-0078D4?logo=microsoftwindows&logoColor=white">
  <img alt="macOS" src="https://img.shields.io/badge/macOS-000000?logo=apple&logoColor=white">
  <a href="LICENSE">
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
  </a>
</p>

---

## 主要功能

- 👁️ **自动记录** — 后台静默运行，实时检测你在用什么应用，自动记录停留时长
- 📊 **时间可视化** — 用分时段的柱状图、使用时间排行榜显示应用使用的时间分布
- 📸 **快照回顾** — 可选择开启屏幕快照，回看时知道你具体在做什么
- 🏷️ **应用分类** — 自定义分类（比如"工作"、"娱乐"、"学习"），按分类统计和查看
- ⏰ **工作时段设置** — 只在设定的工作时间记录，下班后不追踪隐私
- 🔒 **隐私保护** — 自动识别登录页、密码页等敏感内容，跳过截图保护隐私
- ☁️ **多设备支持** — 可选云同步，在多台电脑上查看汇总数据（截图数据本地存储）

## 界面预览

<p align="center">
  <img src="./docs/intro_zh/imgs/today.png" alt="今日总览" width="700"><br/>
  <sub><i>今日总览 · 24 小时堆叠 + 应用排行</i></sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/monthly.png" alt="月统计" width="700"><br/>
  <sub><i>月统计 · 每日柱状 + 月度排行</i></sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/cloud_sync.png" alt="多设备同步" width="700"><br/>
  <sub><i>多设备同步 · 通过 Google Drive 在多台设备间汇总</i></sub>
</p>

## 快速开始

从 [Releases](https://github.com/Tomotsugu-dev/Hindsight/releases) 下载对应平台的安装包并安装。

### Windows

下载 `hindsight_x.y.z_x64-setup.exe`，双击安装即可。

### MacOS

下载 `hindsight_x.y.z_aarch64.dmg`，双击挂载后将 Hindsight 拖入「应用程序」。

> 所有活动数据 / 截图默认仅存本地。如果开启 Google Drive 同步，只会上传活动元数据，**不会上传截图**。

## 未来计划

- [x] 自动识别常用应用并分类，用户可调整分类结果
- [x] 支持自动更新
- [x] 加入 AI 分析功能（分析日概览、周概览、月概览），根据截图内容更精确地识别用户的工作内容
- [ ] 支持生成工作报告（日报、周报、月报）
- [ ] 加入图片加密功能，保护截图隐私
- [ ] 支持更多平台（Linux、移动端）

## License

<p align="center">
  本项目基于 <a href="LICENSE"><b>MIT License</b></a> 开源，欢迎自由使用、修改与分发。<br/>
  <sub>© 2026 Hindsight contributors</sub>
</p>
