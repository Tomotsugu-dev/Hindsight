# Data Management

- [Overview](#overview)
- [Data Export](#data-export)
- [Data Cleanup](#data-cleanup)
- [Defaults](#defaults)

## Overview

Customize data retention and export statistics in several formats for further analysis.

## Data Export

Export daily, weekly, and monthly statistics to a file. Select a date range and format in “Settings → Data → Export usage data”. HTML, Markdown, Excel, and JSON are supported.

## Data Cleanup

- **Automatic cleanup**: Deletes screenshots after their retention period. Activity records and recognized screen text are not automatically cleaned up.
- **Clear screenshots**: Deletes all saved screenshot files, keeping activity records and recognized screen text.
- **Clear data**: Clears activity records, AI summaries, screen text, and chat history from the local database. App configuration, including categories, app groups, and settings, is kept. Cloud data is unaffected.
- **Remove this device**: Clears local data and removes this device's records from the cloud, for when you no longer use the device.

## Defaults

| Setting | Default | Description |
| --- | --- | --- |
| Data folder | The `Hindsight` folder in the system's application data directory | `~/Library/Application Support/Hindsight` on macOS; `%APPDATA%\Hindsight` on Windows |
| Screenshots folder | The `screenshots` folder in the data folder | - |
| AI models folder | The `ai/models` folder in the data folder | - |
| Screenshot retention | 7 days | Configurable from 1–120 days; expired screenshot files are deleted automatically |
| Activity record retention | Forever | Not automatically cleaned up |
