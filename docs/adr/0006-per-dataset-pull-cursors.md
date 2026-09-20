# ADR-0006 · One pull cursor per dataset

- **Date**: 2026-09-19
- **Status**: **Accepted**
- **Related**: PR #45 · PR #47 · `sync::engine::pull::flush_pull` · `docs/design/database.md` (`sync_cursor`)

## Context

`last_pulled_at` records how far this device has got: the modification time of
the last cloud file it handled.

"Clear data" deletes everything except `sync_cursor`. Because the `drive_files`
cursor is kept, the next pull continues from it and does not fetch the files it
has already handled, so the cleared data does not come back on its own.

A pull asks the cloud only for files modified after the cursor. When one of the
optional datasets — AI summaries, chat history, screen memory full text, the
more sensitive ones — is switched off, its files are skipped, and yet
`drive_files` still moves past them.

So when the user wants one of those datasets synced, `drive_files` has to be set
back to 1970 for its files to be listed again.

Merging is idempotent, so normally nothing is visible. But if the user has
cleared local data (`commands::storage::purge_local_data_impl`) before, the
cleared data is merged back from the cloud.

## Decision

One cursor becomes four, each covering one kind of file:

| Cursor | Files |
|---|---|
| `drive_files` | activities, categories, app groups, members, icons, device meta, tombstones |
| `pull.ai_summaries` | AI summaries |
| `pull.chat` | chat |
| `pull.memory` | screen memory full text |

- A stream handles only its own kind of file and advances only its own cursor.
- The switch decides whether a kind is pulled at all; the cursor decides only
  where the stream starts. A missing cursor row reads as 1970: a row is written
  the first time a cursor advances, so there is none right after the upgrade,
  none for a switch that has never been turned on, and none while every file of
  that kind keeps failing. Turning a switch on therefore fills in that dataset's
  history from the beginning.
- While a switch is off its stream does not run and its cursor stays where it
  is; switched on, it continues from there and the backlog follows. Setting a
  cursor back is no longer needed, so `reset_pull_cursor` goes away.
- How a cursor advances is unchanged: only across files that were handled, and
  only to a time before the first file that failed.

## Consequences

- **Benefits**: switching a dataset on affects that dataset only; cleared
  history no longer comes back because of a switch; a stream that fails holds
  back its own cursor and nothing else.
- **Costs**: three more cursor rows. Each round lists files from the earliest
  cursor among the running streams, so a stream that has fallen far behind makes
  the listing longer — a few more file names, the same downloads.

## Data, compatibility, security, and privacy

- **Existing data and migration**: no migration. After the upgrade a dataset
  that is already switched on has no cursor row, so it starts at 1970 and its
  files are downloaded once more; merging is idempotent and the result is the
  same. Screen memory is one file per device per day, so a user who has it on
  downloads a few hundred files that once. Leaving the migration out is
  deliberate: at the moment of the upgrade we do not know how far these three
  streams got, and guessing with `drive_files` would miss the files that were
  marked handled while the memory database was unavailable and never merged.
- **Mixed versions and rollback**: the new version keeps advancing
  `drive_files`, and the old version reads only that one, so going back behaves
  as it does today — passing over the files of a switched-off dataset. The three
  new rows are unknown to the old version and ignored.
- **Irreversible effects**: none.
- **Security and privacy**: no impact; no new data leaves the device.

## Follow-up

- The first pull after a switch is turned on lists every file. Leave it until
  the listing itself is the bottleneck; filtering by file name would first need
  a check of whether Drive's `name contains` only matches prefixes.
