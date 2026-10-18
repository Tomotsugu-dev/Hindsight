# Architecture decision records

Each ADR records one decision: the problem, what we chose, and why. Start a new one from [0000-template.md](0000-template.md). Update this table when you add an ADR or change its status.

- **Status**: 🟢 Accepted · 🟡 Proposed · 🔴 Problem only, no decision yet · ⚪ Superseded
- **Implemented**: ✅ Done · ⬜ Not yet

| ADR | Title | Status | Implemented | 中文 |
| --- | --- | :---: | :---: | :---: |
| [0001](0001-app-deletion-stays-local.md) | Deleting an app only deletes it on this device | 🔴 | ⬜ | [中文](zh/0001-app-deletion-stays-local.md) |
| [0002](0002-derive-auth-key-instead-of-storing-it.md) | Derive the `refresh_token` encryption key instead of storing it | 🟢 | ✅ | |
| [0003](0003-process-paths-overwrite-across-devices.md) | Devices on the same OS overwrite each other's executable paths | 🟢 | ✅ | |
| [0004](0004-retire-app-categories-table.md) | Drop `app_categories` and remove its cloud file | 🟢 | ✅ | |
| [0005](0005-push-rewrites-whole-files.md) | Push rewrites whole cloud files | 🟢 | ✅ | |
| [0006](0006-per-dataset-pull-cursors.md) | One pull cursor per dataset | 🟢 | ✅ | |
| [0007](0007-webdav-backend.md) | Support a WebDAV backend | 🟢 | ✅ | |
| [0008](0008-webdav-manifests.md) | WebDAV pull reads per-device manifests | ⚪ by 0009 | — | |
| [0009](0009-webdav-uploads-and-manifests.md) | WebDAV uploads and manifests | 🟢 | ✅ | |
| [0010](0010-drive-sync-interval.md) | Drive syncs every 5 minutes | 🟢 | ✅ | |
| [0011](0011-backend-switch-and-webdav-account.md) | Switching sync backends and WebDAV accounts | 🟢 | ✅ | |
| [0012](0012-webdav-device-tombstone.md) | WebDAV: remove a device with a tombstone | 🟢 | ⬜ | [中文](zh/0012-webdav-device-tombstone.md) |
| [0013](0013-site-category-rules.md) | Website category rules | 🟢 | ✅ | [中文](zh/0013-site-category-rules.md) |
| [0014](0014-site-icons.md) | Website icons | 🟢 | ✅ | [中文](zh/0014-site-icons.md) |
| [0015](0015-default-category-names-follow-language.md) | Translate default category names once; names no longer sync | 🟡 | ⬜ | [中文](zh/0015-default-category-names-follow-language.md) |
