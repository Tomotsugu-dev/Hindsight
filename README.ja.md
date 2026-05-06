
<p align="center">
  <img src="./src/assets/logo.png" alt="Hindsight" width="200">
</p>

<h1 align="center">Hindsight</h1>

<p align="center">
  <i>ローカルで実行されるコンピュータ活動記録ツール — 1日で使用したアプリを追跡（オプションでクラウド同期対応）</i>
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

## 主な機能

- 👁️ **自動記録** — バックグラウンドで静かに実行され、使用しているアプリをリアルタイムで自動検出・記録します
- 📊 **時間の可視化** — 時間帯別の積み重ねグラフとアプリ使用時間ランキングで使用パターンを表示
- 📸 **スクリーンショットレビュー** — オプションのスクリーンショット機能で、その時何をしていたかを確認できます
- 🏷️ **アプリ分類** — カスタム分類（「仕事」「娯楽」「学習」など）を作成し、分類別に統計情報を表示
- ⏰ **作業時間設定** — 設定した作業時間中のみ記録し、仕事時間外はプライバシーを保護
- 🔒 **プライバシー保護** — ログインページなどの機密コンテンツを自動検出し、スクリーンショットをスキップしてプライバシーを保護
- ☁️ **マルチデバイス対応** — オプションのクラウド同期で複数のコンピュータ間でデータを集約（スクリーンショットはローカルに保存）

## インターフェースプレビュー

<p align="center">
  <img src="./docs/intro_zh/imgs/today.png" alt="Today Overview" width="700"><br/>
  <sub><i>今日の総括 · 24時間積み重ねグラフ + アプリランキング</i></sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/monthly.png" alt="Monthly Statistics" width="700"><br/>
  <sub><i>月間統計 · 日別グラフ + 月間ランキング</i></sub>
</p>

<p align="center">
  <img src="./docs/intro_zh/imgs/cloud_sync.png" alt="Multi-Device Sync" width="700"><br/>
  <sub><i>マルチデバイス同期 · Google Driveを通じた複数デバイス間のデータ集約</i></sub>
</p>

## クイックスタート

[Releases](https://github.com/Tomotsugu-dev/Hindsight/releases)からお使いのプラットフォーム用のインストーラーをダウンロードしてインストールしてください。

### Windows

`hindsight_x.y.z_x64-setup.exe` をダウンロードしてダブルクリックでインストールできます。

### macOS

`hindsight_x.y.z_aarch64.dmg` をダウンロードし、ダブルクリックでマウントしてから Hindsight を「アプリケーション」フォルダにドラッグします。

> すべてのアクティビティデータとスクリーンショットはデフォルトでローカルに保存されます。Google Driveの同期を有効にした場合、アクティビティメタデータのみがアップロードされ、**スクリーンショットはアップロードされません**。

## 今後の計画

- [x] 頻繁に使用されるアプリを自動認識・分類し、ユーザーが調整可能
- [x] オンラインアップデートのサポート
- [x] AI分析機能（日次、週次、月次概要の分析、スクリーンショット内容に基づいた作業内容の正確な識別）
- [ ] 作業レポートの生成（日報、週報、月報）
- [ ] スクリーンショット暗号化機能の追加によるプライバシー保護
- [ ] より多くのプラットフォーム対応（Linux、モバイル）

## ライセンス

<p align="center">
  本プロジェクトは<a href="LICENSE"><b>MITライセンス</b></a>の下でオープンソースとして公開されています。自由に使用、改変、配布できます。<br/>
  <sub>© 2026 Hindsight contributors</sub>
</p>
