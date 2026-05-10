
<p align="center">
  <img src="./src/assets/logo.png" alt="Hindsight" width="200">
</p>

<h1 align="center">Hindsight</h1>

<p align="center">
  <i>あなたのパソコンの日記 — 毎日を、代わりに覚えています。</i>
</p>

<p align="center">
  <a href="README.zh.md">中文</a> · <a href="README.md">English</a> · <a href="README.ja.md">日本語</a>
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

## なぜ Hindsight

深夜にノートパソコンを閉じた瞬間、「今日も一日働いた気がする」のに、何をやり遂げたのか具体的に言えない——そんな経験はありませんか？少し前、この問題を解決しようとトラッキングツールを探し回りましたが、どれも続きませんでした：

- **ActivityWatch** — オープンソースでプライバシー重視、機能リスト上はすべて揃っています。正直な感想：UI に惹かれず、インストールして一度開いてそれきり。
- **WorkReview 系アプリ** — (a) 複数デバイス間での集約と (b) iPhone のスクリーンタイムのような時間単位のタイムライン、両方を満たすものが見つかりませんでした。「午後 3 時に何をしていたか」が一目で分かるズーム可能なビュー、デスクトップでは納得できる形で実装されていません。
- **Toggl / RescueTime / 各種有料 SaaS** — どれもチームや HR 向けの「課金工数」管理のために作られているように感じます。ダッシュボードは情報過多、フローはプロジェクトのタグ付けが前提、データは他社のクラウドに置かれます。「自分自身を振り返る」用途には向きません。

これらの課題を解決するために、Hindsight が生まれました。

## 主な機能

- 📊 **時間の使い道が一目で** — バックグラウンドで自動記録、時間帯別の積み重ねグラフ + アプリランキング；週 / 月単位で集計；カスタム分類（「仕事 / 娯楽 / 学習」など）
- 🤖 **AI 自動日報生成**（新機能）— ローカル LLM がスクリーンショットを読み取り、時間帯別の段落形式の総括を出力
- ☁️ **マルチデバイス集約** — Google Drive で活動データを同期、複数のパソコンから一括閲覧（スクリーンショットは常にローカル保存）
- 🔒 **完全ローカル・プライバシー優先** — データはデフォルトで本機のみ保存；設定した作業時間のみ記録；ログイン / パスワード画面のスクリーンショットを自動スキップ

## インターフェースプレビュー

<p align="center">
  <video src="https://github.com/user-attachments/assets/68001c5d-f602-40de-8965-b9f46547da39" controls muted autoplay loop playsinline width="800"></video><br/>
  <sub><i><b>アプリプレビュー</b> · Hindsight の主要な操作を 30 秒で</i></sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/today.png" alt="Today Overview" width="800"><br/>
  <sub><i><b>今日の総括</b> · 24時間積み重ねグラフ × アプリランキング、今日の時間の使い道と仕事 / 学習リズムを一目で把握</i></sub>
</p>

<table align="center">
  <tr>
    <td align="center" width="50%">
      <img src="./docs/intro_zh/imgs/weekly.png" alt="Weekly Statistics"><br/>
      <sub><i><b>週間統計</b> · 7日間の総時間を棒グラフで比較、週のよく使ったアプリランキング付き</i></sub>
    </td>
    <td align="center" width="50%">
      <img src="./docs/intro_zh/imgs/monthly.png" alt="Monthly Statistics"><br/>
      <sub><i><b>月間統計</b> · 日別グラフ × 月間ランキング、長期的な作業リズムを把握</i></sub>
    </td>
  </tr>
  <tr>
    <td align="center" width="50%">
      <img src="./docs/intro_zh/imgs/ai_summary.png" alt="AI Summary"><br/>
      <sub><i><b>AI 自動日報</b> · ローカル LLM が時間帯別にスクリーンショットを読み取り、段落形式の総括を出力；スクリーンショットは常にローカル</i></sub>
    </td>
    <td align="center" width="50%">
      <img src="./docs/intro_zh/imgs/cloud_sync.png" alt="Multi-Device Sync"><br/>
      <sub><i><b>マルチデバイス集約</b> · Google Drive で活動メタデータを同期、複数の端末から一括閲覧；スクリーンショットは常にローカル</i></sub>
    </td>
  </tr>
</table>

<p align="center">
  <img src="./docs/intro_zh/imgs/ai_chatbot.png" alt="AI Assistant" width="800"><br/>
  <sub><i><b>AI アシスタント</b> 🚧 近日公開 · 自然言語で活動記録に質問、「先週コードを何時間書いた？」「どの時間帯が一番集中力が散漫？」</i></sub>
</p>

## クイックスタート

[Releases](https://github.com/Tomotsugu-dev/Hindsight/releases) からお使いのプラットフォーム用のインストーラーをダウンロードしてインストールしてください。

### Windows

`hindsight_x.y.z_x64-setup.exe` をダウンロードしてダブルクリックでインストールできます。

> ⚠️ **初回実行時に「WindowsによってPCが保護されました」と表示されます** — インストーラーはまだ EV コード署名証明書を取得していないため、SmartScreen にブロックされます。「詳細情報」→「実行」をクリックしてインストールを続行してください。

### macOS

`hindsight_x.y.z_universal.dmg`（Apple Silicon + Intel ユニバーサルバイナリ）をダウンロードし、ダブルクリックでマウントしてから Hindsight を「アプリケーション」フォルダにドラッグします。Apple Developer 証明書による署名と公証済みのため、Gatekeeper の警告なしでそのまま開けます。

> すべてのアクティビティデータとスクリーンショットはデフォルトでローカルに保存されます。Google Drive の同期を有効にした場合、アクティビティメタデータのみがアップロードされ、**スクリーンショットはアップロードされません**。

## 今後の計画

- [x] 頻繁に使用されるアプリを自動認識・分類し、ユーザーが調整可能
- [x] オンラインアップデートのサポート
- [x] AI分析機能（日次、週次、月次概要の分析、スクリーンショット内容に基づいた作業内容の正確な識別）
- [ ] 作業レポートの生成（日報、週報、月報）
- [ ] スクリーンショット暗号化機能の追加によるプライバシー保護
- [ ] より多くのプラットフォーム対応（Linux、モバイル）

## 技術スタック

| カテゴリ | 技術 |
|---|---|
| デスクトップフレームワーク | [Tauri 2](https://tauri.app/) |
| フロントエンド | React 19 · TypeScript · Vite |
| バックエンド | Rust · Tokio · SQLite · reqwest |
| AI 推論 | [llama.cpp](https://github.com/ggml-org/llama.cpp) · Qwen2.5-VL / Qwen3-VL · OpenAI 互換 API |
| 同期 | Google Drive API |

## ライセンス

<p align="center">
  本プロジェクトは<a href="LICENSE"><b>MITライセンス</b></a>の下でオープンソースとして公開されています。自由に使用、改変、配布できます。<br/>
  <sub>© 2026 Hindsight contributors</sub>
</p>
