
<p align="center">
  <img src="./src/assets/logo.png" alt="Hindsight" width="200">
</p>

<h1 align="center">Hindsight</h1>

<p align="center">
  <i>Your computer's diary — it remembers every day for you.</i>
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

- 📊 **See where your time goes** — Background auto-tracking with hourly histograms + app rankings; weekly / monthly aggregation; customizable categories ("Work / Entertainment / Learning")
- 🤖 **AI auto-generated daily report** (new) — Local LLM reads your screenshots and writes a per-segment summary
- ☁️ **Multi-device aggregation** — Optional Google Drive sync of activity data; view across multiple computers (screenshots stay local)
- 🔒 **Local-first, privacy-first** — Data stays on your machine by default; records only during your set work hours; auto-skips screenshots of login / password pages

## Interface Preview

<p align="center">
  <img src="./docs/intro_zh/imgs/today.png" alt="Today Overview" width="800"><br/>
  <sub><i><b>Today Overview</b> · 24-hour stacked histogram × app leaderboard — see where your day went and your work / study rhythm at a glance</i></sub>
</p>

<table align="center">
  <tr>
    <td align="center" width="50%">
      <img src="./docs/intro_zh/imgs/weekly.png" alt="Weekly Statistics"><br/>
      <sub><i><b>Weekly Stats</b> · 7-day total-time bar comparison with weekly top-app ranking</i></sub>
    </td>
    <td align="center" width="50%">
      <img src="./docs/intro_zh/imgs/monthly.png" alt="Monthly Statistics"><br/>
      <sub><i><b>Monthly Stats</b> · Daily bars × monthly ranking — see your long-term work rhythm</i></sub>
    </td>
  </tr>
  <tr>
    <td align="center" width="50%">
      <img src="./docs/intro_zh/imgs/ai_summary.png" alt="AI Summary"><br/>
      <sub><i><b>AI auto-daily-report</b> · Local LLM reads screenshots per time segment and outputs a paragraph-style summary; screenshots stay local</i></sub>
    </td>
    <td align="center" width="50%">
      <img src="./docs/intro_zh/imgs/cloud_sync.png" alt="Multi-Device Sync"><br/>
      <sub><i><b>Multi-device aggregation</b> · Google Drive syncs activity metadata across machines; screenshots always stay local</i></sub>
    </td>
  </tr>
</table>

<p align="center">
  <img src="./docs/intro_zh/imgs/ai_chatbot.png" alt="AI Assistant" width="800"><br/>
  <sub><i><b>AI Assistant</b> 🚧 Coming soon · Ask your activity log in natural language: "How many hours did I code last week?" "When am I most easily distracted?"</i></sub>
</p>

## Quick Start

Download the installer for your platform from [Releases](https://github.com/Tomotsugu-dev/Hindsight/releases) and install it.

### Windows

Download `hindsight_x.y.z_x64-setup.exe` and double-click to install.

> ⚠️ **First launch will trigger "Windows protected your PC"** — the installer is not yet signed with an EV code-signing certificate, so SmartScreen will block it. Click "More info" → "Run anyway" to continue.

### macOS

Download `hindsight_x.y.z_aarch64.dmg`, double-click to mount, then drag Hindsight into the Applications folder.

> ⚠️ **First open will show "cannot verify developer" or "is damaged"** — the app has not yet been notarized with Apple, so macOS Gatekeeper blocks it. Run the following in Terminal to clear the quarantine flag, then open normally:

```bash
sudo xattr -rd com.apple.quarantine "/Applications/Hindsight.app"
```

> All activity data and screenshots are stored locally by default. If you enable Google Drive sync, only activity metadata will be uploaded, **screenshots will not be uploaded**.

## Future Roadmap

- [x] Auto-identify and categorize frequently-used apps, with user adjustment capability
- [x] Support for auto-updates
- [x] AI analysis features (analyze daily, weekly, and monthly overviews, identify work content more accurately based on screenshot content)
- [ ] Generate work reports (daily, weekly, monthly)
- [ ] Add screenshot encryption to protect privacy
- [ ] Support for more platforms (Linux, mobile)

## Tech Stack

| Category | Technology |
|---|---|
| Desktop framework | [Tauri 2](https://tauri.app/) |
| Frontend | React 19 · TypeScript · Vite |
| Backend | Rust · Tokio · SQLite · reqwest |
| AI inference | [llama.cpp](https://github.com/ggml-org/llama.cpp) · Qwen2.5-VL / Qwen3-VL · OpenAI-compatible API |
| Sync | Google Drive API |

## License

<p align="center">
  This project is open source under the <a href="LICENSE"><b>MIT License</b></a>. Feel free to use, modify, and distribute.<br/>
  <sub>© 2026 Hindsight contributors</sub>
</p>
