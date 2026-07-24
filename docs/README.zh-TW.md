
<p align="center">
  <img src="../src/assets/logo.png" alt="Hindsight" width="200">
</p>

<h1 align="center">Hindsight</h1>

<p align="center">
  <i>你的電腦日記 — 它替你記得每一天。</i>
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

## 為什麼是 Hindsight

你是不是也常常凌晨闔上電腦，覺得自己「忙了一整天」，卻說不上今天到底做成了什麼？前陣子我想找個時間追蹤工具來解決這個問題，市面上挑了一圈都沒用下去：

- **[ActivityWatch](https://github.com/ActivityWatch/activitywatch)** — 開源、隱私優先，功能上挑不出毛病。但老實說，它的介面沒什麼吸引力，裝完打開看過一次，之後就再也沒點開過。
- **[WorkReview](https://github.com/wm94i/Work-Review)** — 我想要兩件事同時滿足：一是能跨裝置彙總，二是像 iPhone「螢幕使用時間」那樣按小時縮放的時間軸，讓我直接看到「下午 3 點我在做什麼」。桌面端沒有一款做到讓我滿意。
- **[Toggl](https://toggl.com) / [RescueTime](https://www.rescuetime.com) / 各種付費 SaaS** — 這些本質上是給團隊和 HR 算「計費工時」用的：儀表板資訊密集，流程繞著專案貼標籤轉，資料還要傳到對方的雲端。我要的是「自己跟自己覆盤」，方向完全對不上。

為了解決以上這些問題，Hindsight 應運而生。

## 介面預覽

<p align="center">
  <video src="https://github.com/user-attachments/assets/e6349610-a742-4ba2-abca-412f00b1673c" controls muted autoplay loop playsinline width="800"></video>
</p>
<p align="center">
  <sub><i><b>軟體預覽</b> · 1 分鐘看清 Hindsight 的核心互動</i></sub>
</p>

<p align="center">
  <img src="./intro_zh/imgs/today.png" alt="今日總覽" width="800"><br/>
  <sub><i><b>今日總覽</b> · 24 小時分時段堆疊圖 × 應用程式排行榜，一眼看清今天的時間去向，工作學習節奏</i></sub>
</p>

<p align="center">
  <img src="./intro_zh/imgs/monthly.png" alt="月統計" width="800"><br/>
  <sub><i><b>月統計</b> · 每日長條 × 月度排行，看清長期工作節奏</i></sub>
</p>

<p align="center">
  <img src="./intro_zh/imgs/ai_summary.png" alt="AI 摘要" width="800"><br/>
  <sub><i><b>AI 自動寫日報</b> · 本機 LLM 按時段看截圖，輸出段落式摘要；截圖始終留在本機</i></sub>
</p>

## 主要功能

- 📊 **看清時間花在哪** — 背景自動記錄，分時段長條圖 + 應用程式排行；按週 / 月彙總；可自訂分類（「工作 / 娛樂 / 學習」）
- 🤖 **AI 自動寫日報**（新）— 本機 LLM 讀你的截圖，按時段寫出段落式摘要
- ☁️ **多裝置彙總** — 可選 Google Drive 同步活動資料，多台電腦一處檢視（截圖始終留在本機）
- 🔒 **完全本機、隱私優先** — 資料預設僅存本機；只在設定的工作時段記錄；自動跳過登入 / 密碼頁截圖

## 快速開始

從 [Releases](https://github.com/Tomotsugu-dev/Hindsight/releases) 下載對應平台的安裝檔並安裝。

### Windows

下載 `hindsight_x.y.z_x64-setup.exe`，按兩下安裝即可。

> ⚠️ **首次執行會跳出「Windows 已保護你的電腦」** — 安裝檔尚未購買 EV 程式碼簽章憑證，會被 SmartScreen 攔下。點選「其他資訊」→「仍要執行」即可繼續安裝。

### MacOS

下載 `hindsight_x.y.z_universal.dmg`（Apple Silicon + Intel 通用二進位檔），按兩下掛載後將 Hindsight 拖入「應用程式」即可正常開啟——應用程式已接入 Apple 開發者憑證簽署 + 公證，不會再觸發 Gatekeeper 警告。

> 所有活動資料 / 截圖預設僅存本機。如果開啟 Google Drive 同步，只會上傳活動中繼資料，**不會上傳截圖**。

## 技術堆疊

| 類別 | 技術 |
|---|---|
| 桌面框架 | [Tauri 2](https://tauri.app/) |
| 前端 | React 19 · TypeScript · Vite |
| 後端 | Rust · Tokio · SQLite · reqwest |
| AI 推論 | [llama.cpp](https://github.com/ggml-org/llama.cpp) · Qwen2.5-VL / Qwen3-VL · OpenAI 相容 API |
| 同步 | Google Drive API |

## License

<p align="center">
  本專案基於 <a href="../LICENSE"><b>MIT License</b></a> 開源，歡迎自由使用、修改與散布。<br/>
  <sub>© 2026 Hindsight contributors</sub>
</p>
