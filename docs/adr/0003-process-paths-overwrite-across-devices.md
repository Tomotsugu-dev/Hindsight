# ADR-0003 · Devices on the same OS overwrite each other's executable paths

- **Date**: 2026-09-14 (problem recorded) · 2026-09-16 (decision)
- **Status**: **Accepted**
- **Related**: commit `9af557a` (table) · `a694cda` (sync columns) · `3636eb8` (into sync) · `8a65ac3` (cross-OS filter) · ADR-0004 (same retirement pattern) · ADR-0005

## Context

### What the user sees

Two computers on the same OS have the same app installed in different
places. After syncing for a while, **both computers hold one of the two
install paths**, and while both are using the app the record flips between
the two paths without end. Reproduced with the end-to-end sync harness.

Windows hits this easily: per-user installs (Slack, Discord, VS Code user
setup) carry the login name in the path.

```
Computer A  C:\Users\alice\AppData\Local\slack\slack.exe
Computer B  C:\Users\bob\AppData\Local\slack\slack.exe
```

### The chain

The synced table is `process_paths`. Each device publishes its **whole table**
as one cloud file; a peer merges it **row by row on the process name**, newer
timestamp wins. The table allows one row per process name, so only one of
the two paths can survive.

1. **A records its own path.** Capture calls `process_paths::upsert` with
   `slack.exe → C:\Users\alice\...`. The path differs from the stored one, so an
   outbox row is queued.
2. **A publishes the whole table** as `device.A.process_paths.json`.
3. **B pulls the file and passes the cross-OS gate.** Both are Windows.
4. **B merges by process name.** A's timestamp is newer, so
   `ON CONFLICT(process_name) DO UPDATE SET exe_path = excluded.exe_path`
   **replaces B's `slack.exe` with A's path**.
5. **B uses Slack again and writes its own path back.** The stored path differs
   from the local one, so another outbox row, another upload.
6. **A pulls and gets B's path.** Back to step 4 in the other direction.

### What the path is for

The path has one reader: the last level of icon lookup, which extracts the
icon from the local executable. Icons themselves travel in their own file,
`device.<id>.icons.json`: a device that has shown an app has extracted its
icon and pushed the bytes, and a peer that never ran the app gets the icon
from those bytes, never from the path. A peer's path would only help when the
peer failed to extract, the two devices share an OS and an install location,
and this device never ran the app. That case has not been observed.

## Decision

**Take `process_paths` out of sync.** It goes back to being a local cache:

1. Push stops publishing `device.<id>.process_paths.json`, and `upsert` stops
   queueing outbox rows.
2. Pull stops merging the file; the name is skipped like any other unknown
   name.
3. On the first push after each launch, the device deletes its own
   `process_paths.json` from the cloud, the same way ADR-0004 removes
   `app_categories.json`.
4. The cross-OS gate and the `devices.os` refresh, which existed only for this
   file, go with it.

## Alternatives

| Option | Why not |
|---|---|
| **Stop syncing the table (chosen)** | — |
| Add an owner column; a peer's row never replaces this device's own; push only own rows | Stops the overwrite, but keeps a column, a migration, a three-way merge and a push filter alive for the one case described above, which nobody has seen |
| Key the table by `(device_id, process_name)` | Same as above, plus rebuilding the table |
| Merge with `DO NOTHING` | Stops the overwrite, but push keeps forwarding peers' rows, and nothing can tell a forwarded row from an own one |

## Consequences

**The overwrite and the flip-flop stop** on every upgraded device, and one
sync entity, its cross-OS gate and its e2e tests disappear.

**Fewer paths leave the machine.** Executable paths carry the login name
(`C:\Users\alice\...`); they no longer reach the cloud at all.

**Devices on older versions keep overwriting** until they upgrade; the fix
takes effect only once every device in the account runs it.

**Cloud cleanup reaches only each upgraded device's own file**, as in
ADR-0004.

## Data, compatibility, security, and privacy

- **Existing data and migration**: none. The table keeps its shape; rows
  overwritten before the fix stay until the app is used again locally, which
  writes the right path back. `updated_at` stays but no longer means
  anything.
- **Mixed versions and rollback**: an older version resumes publishing and
  merging on its own; nothing is lost either way. Outbox rows with entity
  `process_path` left by older versions are logged and deleted like any other
  unknown entity.
- **Irreversible effects**: the cloud file is deleted. It is a copy of local
  data that nothing reads.
- **Security and privacy**: improved; see above.

## Follow-up

- End-to-end tests: a peer's `process_paths.json` is no longer merged, this
  device no longer publishes one, and the first push after launch deletes the
  stale one.
- The cloud cleanup is temporary and marked `TODO(ADR-0003)`; remove it once
  active devices have upgraded.
- `docs/design/database.md`: update the `process_paths` touchpoints.
