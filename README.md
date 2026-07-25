<p align="center">
  <img src="./src/assets/logo.png" alt="Hindsight" width="180">
</p>

<h1 align="center">Hindsight</h1>

<p align="center">
  <strong>A local-first computer activity log & AI review tool</strong><br/>
  Automatically records app and window activity and shows where your time went by day, week, and month — no manual timers, no project tags to maintain.
</p>

<p align="center">
  <a href="docs/README.zh.md">简体中文</a> · <a href="docs/README.zh-TW.md">繁體中文</a> · <a href="README.md">English</a> · <a href="docs/README.ja.md">日本語</a> · <a href="docs/README.pt.md">Português</a>
</p>

<p align="center">
  <a href="https://github.com/Tomotsugu-dev/Hindsight/releases">
    <img alt="GitHub Release" src="https://img.shields.io/github/v/release/Tomotsugu-dev/Hindsight?color=blue&logo=github">
  </a>
  <a href="https://github.com/Tomotsugu-dev/Hindsight/stargazers">
    <img alt="GitHub Stars" src="https://img.shields.io/github/stars/Tomotsugu-dev/Hindsight?style=flat&logo=github&color=yellow">
  </a>
  <a href="https://github.com/Tomotsugu-dev/Hindsight/actions/workflows/ci.yml">
    <img alt="CI" src="https://github.com/Tomotsugu-dev/Hindsight/actions/workflows/ci.yml/badge.svg">
  </a>
  <a href="LICENSE">
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
  </a>
</p>
<p align="center">
  <img alt="Windows" src="https://img.shields.io/badge/Windows-0078D4?logo=microsoftwindows&logoColor=white">
  <img alt="macOS" src="https://img.shields.io/badge/macOS-000000?logo=apple&logoColor=white">
</p>

<p align="center">
  <a href="https://github.com/Tomotsugu-dev/Hindsight/releases"><b>Download latest</b></a> ·
  <a href="#interface-preview">Interface Preview</a> ·
  <a href="#key-features">Key Features</a> ·
  <a href="#quick-start">Quick Start</a>
</p>

---

## Interface Preview

<p align="center">
  <video src="https://github.com/user-attachments/assets/fe05771d-718a-418b-80a1-12fd76a826ab" controls muted autoplay loop playsinline width="800"></video>
</p>
<p align="center">
  <sub><b>App preview</b> · Hindsight's core interactions in 1 minute</sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/daily.png" alt="Daily stats" width="800"><br/>
  <sub><b>Daily stats</b> · 24-hour stacked timeline × app / category rankings — see where today went at a glance</sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/app_detail.png" alt="App detail" width="800"><br/>
  <sub><b>App detail</b> · Click any app to see what you were actually doing inside it</sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/weekly.png" alt="Weekly stats" width="800"><br/>
  <sub><b>Weekly stats</b> · Your whole week at a glance</sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/monthly.png" alt="Monthly stats" width="800"><br/>
  <sub><b>Monthly stats</b> · Daily activity bars with app / category rankings — see where this month's time really went</sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/monthly_cal.png" alt="Monthly breakdown" width="800"><br/>
  <sub><b>Monthly breakdown</b> · Time share by category, with totals, daily average, and change vs. last month</sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/ai_summary.png" alt="AI Summary" width="800"><br/>
  <sub><b>AI daily report</b> · A local or cloud model rolls up each segment of your day into a written report</sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/ai_chatbot.png" alt="AI Chat" width="800"><br/>
  <sub><b>AI chat</b> · Just ask, e.g. "How much time did I spend on X this month?"</sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/cloud_sync.png" alt="Multi-device sync" width="800"><br/>
  <sub><b>Multi-device sync</b> · For people who use more than one computer</sub>
</p>

## Key Features

- 📊 **See where your time goes** — Automatic background tracking with per-segment histograms + app rankings; daily / weekly / monthly rollups, click any app for title-level detail; customizable categories ("Work / Entertainment / Learning")
- 🤖 **AI daily report** — A local model writes up each segment of your day; with Auto summary on, yesterday's daily and last week's weekly reports fill in automatically
- 💬 **AI chat** — Ask "What did I do today?" or "How long did I spend on project X this month?" — answered from your own records
- 🔍 **Screen memory search** — Find any text that ever appeared on your screen and jump to the screenshot and moment (screenshots and OCR are off by default, enable as needed)
- ☁️ **Multi-device aggregation** — Optional Google Drive sync of activity data; view all your computers in one place (screenshots never leave the device)
- 🔒 **Local & privacy-first** — Data stays on your machine by default

## Why Hindsight

Have you ever closed the laptop at midnight feeling like you "worked all day" but couldn't say what you actually got done? A while back I went hunting for a tracker to fix exactly that. Tried a bunch — none of them stuck:

- **[ActivityWatch](https://github.com/ActivityWatch/activitywatch)** — open-source, privacy-first, technically ticks all the right boxes. Honest take: the UI just doesn't pull me in. I'd install it, look at it once, never open it again.
- **[WorkReview](https://github.com/wm94i/Work-Review)** — couldn't find one with both (a) cross-device visibility and (b) an hourly timeline like iPhone's Screen Time. I really wanted that "what was I doing at 3pm" zoomable view for desktop, and nothing had it the way I wanted.
- **[Toggl](https://toggl.com) / [RescueTime](https://www.rescuetime.com) / paid SaaS** — these feel built for teams and HR-style "billable hours" tracking. The dashboards are dense, the flow is project-tagging-first, and the data lives on someone else's servers. Wrong tool for "personal awareness."

To fix exactly these gaps, I built Hindsight.

## Quick Start

Download the installer for your platform from [Releases](https://github.com/Tomotsugu-dev/Hindsight/releases) and install it.

### Windows

Download `hindsight_x.y.z_x64-setup.exe` and double-click to install.

> ⚠️ **First launch will trigger "Windows protected your PC"** — the installer is not yet signed with an EV code-signing certificate, so SmartScreen will block it. Click "More info" → "Run anyway" to continue.

### macOS

Download `hindsight_x.y.z_universal.dmg` (Apple Silicon + Intel universal binary), double-click to mount, then drag Hindsight into the Applications folder. The app is signed with an Apple Developer certificate and notarized, so it opens normally without any Gatekeeper warning.

> All activity data and screenshots are stored locally by default. If you enable Google Drive sync, only activity metadata will be uploaded, **screenshots will not be uploaded**.

## License

<p align="center">
  This project is open source under the <a href="LICENSE"><b>MIT License</b></a>. Feel free to use, modify, and distribute.<br/>
  <sub>© 2026 Hindsight contributors</sub>
</p>
