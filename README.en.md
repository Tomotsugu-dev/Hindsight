
<p align="center">
  <img src="./src/assets/logo.png" alt="Hindsight" width="200">
</p>

<h1 align="center">Hindsight</h1>

<p align="center">
  <i>A local activity tracker for your computer — Track the apps you used throughout the day (with optional cloud sync)</i>
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

## Key Features

- 👁️ **Automatic Tracking** — Runs silently in the background, automatically detects and logs your app usage in real time
- 📊 **Time Visualization** — View app usage patterns with hourly histograms and ranking leaderboards
- 📸 **Screenshot Review** — Optional screen snapshots let you see exactly what you were doing at any time
- 🏷️ **App Classification** — Create custom categories (e.g., "Work", "Entertainment", "Learning") and view statistics by category
- ⏰ **Work Hours Settings** — Record only during your set work hours, protect your privacy outside work time
- 🔒 **Privacy Protection** — Automatically detects sensitive content like login pages and password fields, skips screenshots to protect your privacy
- ☁️ **Multi-Device Support** — Optional cloud sync to view aggregated data across multiple computers (screenshots remain local)

## Interface Preview

<p align="center">
  <img src="./docs/intro_zh/imgs/今日总览.png" alt="Today Overview" width="700"><br/>
  <sub><i>Today Overview · 24-hour stacked histogram + app ranking</i></sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/月统计.png" alt="Monthly Statistics" width="700"><br/>
  <sub><i>Monthly Statistics · Daily histogram + monthly ranking</i></sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/应用分类.png" alt="App Classification" width="700"><br/>
  <sub><i>App Classification · Custom colors and icons, unclassified apps are easily visible</i></sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/多设备同步.png" alt="Multi-Device Sync" width="700"><br/>
  <sub><i>Multi-Device Sync · Aggregate data across devices via Google Drive</i></sub>
</p>

## Quick Start

Download the installer for your platform from [Releases](https://github.com/Tomotsugu-dev/Hindsight/releases) and install it.

### Windows

Download `hindsight_x.y.z_x64-setup.exe` and double-click to install.

### macOS

<--Placeholder-->

> All activity data and screenshots are stored locally by default. If you enable Google Drive sync, only activity metadata will be uploaded, **screenshots will not be uploaded**.

## Future Roadmap

- [x] Auto-identify and categorize frequently-used apps, with user adjustment capability
- [x] Support for auto-updates
- [x] AI analysis features (analyze daily, weekly, and monthly overviews, identify work content more accurately based on screenshot content)
- [ ] Generate work reports (daily, weekly, monthly)
- [ ] Add screenshot encryption to protect privacy
- [ ] Support for more platforms (Linux, mobile)

## License

<p align="center">
  This project is open source under the <a href="LICENSE"><b>MIT License</b></a>. Feel free to use, modify, and distribute.<br/>
  <sub>© 2026 Hindsight contributors</sub>
</p>
