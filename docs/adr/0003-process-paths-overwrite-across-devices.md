# ADR-0003 · Devices on the same OS overwrite each other's executable paths

- **Date**: 2026-09-14 (problem recorded) · 2026-09-16 (decision)
- **Status**: **Accepted**
- **Related**: commit `9af557a` (table) · `a694cda` (sync columns) · `3636eb8` (into sync) · `8a65ac3` (cross-OS filter) · `sync::engine::pull::merge_process_paths` · ADR-0005

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

Icons travel in a separate file, `device.<id>.icons.json`, outside this chain.

## Decision

**Record which device wrote each row, and never let a peer's row replace this
device's own.** Concretely:

1. `process_paths` gains a `device_id` column. One row per process name stays;
   the key does not change.
2. `upsert` stamps the row with this device's id.
3. `merge_process_paths` writes a peer's row under the peer's id (known from
   the file name) and **skips any row this device wrote itself**.
4. `build_process_paths` publishes **only this device's rows**, so paths
   received from others are no longer forwarded.

Rows that exist before the migration are treated as this device's own.

## Alternatives

| Option | Why not |
|---|---|
| **Add an owner column, local rows win (chosen)** | — |
| Key the table by `(device_id, process_name)` | Keeps every device's path for every app, but this device only needs its own; the extra rows would only add candidates for icon extraction. Not worth rebuilding the table |
| Stop syncing the table | Simplest, and the same shape as ADR-0004. Loses icon extraction from a peer's path for apps this device has never run, and that value was never measured |
| Merge with `DO NOTHING` and no owner column | Stops the overwrite, but push would still forward peers' rows, and nothing could tell a forwarded row from an own one |

## Consequences

**A device's own path can no longer be overwritten by sync**, and the
flip-flop stops: a device never publishes a row it did not write, and never
accepts a row for an app it has recorded itself.

**Peers' paths still arrive for apps this device has never run**, so level 3
of icon lookup keeps its chance on the same OS. Such a path may not exist
locally; extraction then returns nothing, as today.

**Devices on older versions keep overwriting** until they upgrade; the fix
takes effect only once every device in the account runs it.

## Data, compatibility, security, and privacy

- **Existing data and migration**: one migration adds the column with an
  empty default; rows with the empty value count as this device's own. Rows
  already overwritten cannot be told apart from the data: there is no owner
  column, and the path-shape cleanup migration v11 used against cross-OS
  pollution does not apply when both paths have the same shape. They are
  corrected the next time the app is used locally.
- **Mixed versions and rollback**: the cloud file keeps its fields; the owner
  is the file's device prefix, which every version already writes. An older
  version ignores the new column, so rolling back loses nothing.
- **Irreversible effects**: none.
- **Security and privacy**: unchanged; the same paths are synced as before.

## Follow-up

- Before the fix, a failing end-to-end test: two devices on the same OS
  record different paths for one app; after a sync round each still holds its
  own. It is red today.
- With the owner known, the cross-OS gate and the `devices.os` refresh it
  depends on are no longer needed for correctness. Decide separately whether
  to keep them as a filter.
