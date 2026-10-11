# Cloud Sync

- [Overview](#overview)
- [Supported Protocols](#supported-protocols)
- [Data Included in Sync](#data-included-in-sync)
- [Defaults](#defaults)

## Overview

Cloud Sync shares activity records, app settings, and other data across devices, bringing together a complete record of computer activity across platforms and devices.

## Supported Protocols

Hindsight currently supports:

- **WebDAV**: Sync through a self-hosted or third-party WebDAV server, such as Nextcloud or Nutstore.
- **Google Drive**: Sync through Google Drive using a Google account.

## Data Included in Sync

| Data | Synced? | Description |
| --- | --- | --- |
| Activity records (apps, window titles, times, website domains) | ✅ Yes | - |
| App settings (app and website category rules) | ✅ Yes | - |
| AI summaries, chat history, screen memory full text | ⚙️ Optional | Enable each separately in settings |
| Screenshot images | ❌ No | Always stay on the local device |

## Defaults

| Setting | Default | Description |
| --- | --- | --- |
| Cloud Sync | Off | Requires configuration |
| Sync interval | About 5 minutes | Uploads and downloads automatically every 300 seconds in the background; “Sync now” is also available |
| AI summary text | Off | Synced only when “AI summary text” is enabled |
| Chat history | Off | Synced only when “Chat history” is enabled |
| Screen memory (full text) | Off | Recognized text is synced only when “Screen memory (full text)” is enabled |
| Screenshot images | Not synced | Always stay on the local device |
