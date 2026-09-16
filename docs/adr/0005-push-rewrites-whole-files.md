# ADR-0005 · Push rewrites whole cloud files; an outbox row only marks a file dirty

- **Date**: 2026-09-16 (implemented on 2026-05-03 in `a694cda`; recorded retrospectively)
- **Status**: **Accepted**
- **Related**: `repo::outbox::enqueue` · `sync::engine::push::group_outbox` · `docs/design/database.md` (`sync_outbox`)

## Context

Every device publishes its own data to the cloud as a set of files under its
own prefix, and other devices merge those files row by row. Each table is one
file, except activities, which are one file per local day:

```
device.<id>.categories.json
device.<id>.app_groups.json
device.<id>.app_group_members.json
device.<id>.process_paths.json
device.<id>.icons.json
device.<id>.meta.json
device.<id>.activities.<YYYY-MM-DD>.ndjson
```

A local write has to reach the cloud eventually, even if the device is
offline at that moment. Something has to remember that a push is due.

## Decision

**An outbox row means "this file must be rewritten", nothing more.** Every
local write inserts one row into `sync_outbox` in the same transaction. Push
groups the pending rows by the file they map to, rebuilds each such file in
full from the tables, uploads it, and deletes the rows.

Consequently:

- The `payload` column is read for one entity only, `activity`, where
  `localDate` says which day's file is dirty. For every other entity, one table
  is one file, so the `entity` column alone identifies it and the payload is
  ignored.
- The `op` column is never read. A deletion is an ordinary upsert whose row
  carries `deleted_at`; the rebuilt file simply contains the tombstone.
- Data never travels through the outbox. What is uploaded is always the current
  table content at push time.

## Alternatives

| Option | Why not |
|---|---|
| **Rewrite whole files (chosen)** | — |
| Push each changed row on its own | The sender's retry works either way; the problems are elsewhere. Drive has no "change one row" and no append: a row is either its own file, and a folder of tens of thousands of files makes a first sync take hours of API requests, or it is appended to a file, which Drive can only do by re-uploading the whole file. And when a peer fails to merge one record (its category has not arrived yet, say), the sender never learns of it while the peer's cursor has already moved past the record. With whole files, the next rewrite carries the row again and the peer repairs itself |
| Append a change log per device | Same two problems, plus the log grows without bound until it is compacted into a snapshot, which is the chosen option again |

## Consequences

**Traffic scales with file size, not with the change.** Measured on the
maintainer's database (2026-09-16):

| File | Size | Rewritten when |
|---|---|---|
| `icons.json` | 8.2 MB (209 icons, base64) | a new icon is extracted locally; rare after setup |
| `activities.<day>.ndjson` | ~190 KB (750 rows, grows through the day) | **every time a session ends**, so on a busy day nearly every 30-second push cycle |
| groups + members | 26 KB + 24 KB | a group changes or a new app appears |
| `process_paths.json` | 13 KB | a path changes |
| `categories.json` | 4 KB | a category changes |

The day file dominates: tens to a hundred megabytes of upload on a busy day,
and each peer downloads it again on every pull. Fine on a home connection,
noticeable on a mobile hotspot.

**Idempotent by construction.** A file can be uploaded or merged any number
of times; row-level last-write-wins on `updated_at` decides each row.

**The outbox schema over-promises.** `op` and, for six of seven entities,
`payload` are stored and never read, which misleads readers into thinking
rows carry data. `database.md` documents this; the columns stay for now.

## Data, compatibility, security, and privacy

- **Existing data and migration**: none; this records the current behaviour.
- **Mixed versions and rollback**: no impact.
- **Irreversible effects**: none.
- **Security and privacy**: unchanged; the same file contents are uploaded either way.

## Follow-up

- Stop storing meaningless payloads: `enqueue` could take the day only for
  `activity` and store `{}` for the rest. Small refactor; about 25 call sites
  across eight files.
- Revisit the day-file rewrite if users report traffic problems. Options then
  are pushing at most once per N minutes, or splitting the day file by hour.
