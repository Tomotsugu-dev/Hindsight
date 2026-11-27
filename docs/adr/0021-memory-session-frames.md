# ADR-0021 · Remove repeated data from the screen memory: `session_lines` becomes `session_frames`

- **Date**: 2026-11-28
- **Status**: Accepted
- **Related**: ADR-0016

The screen memory database (`hindsight-memory.<uid>.sqlite`) stores the same data more than once. Each text line in `session_lines` repeats the path and the time of its screenshot, and the text itself is also in `text_sessions.text`. We add an integer primary key `id` to `frames`, and drop `local_date`, which `ts` already gives. `session_frames` replaces `session_lines`: it has one row for each (session, screenshot), with the line of the session text where that screenshot's text starts. The text is kept only in `text_sessions.text`. The first time the new version starts, it changes the tables in one transaction in the background. Then, if there is room, it runs `VACUUM`. On this computer, the memory database went from 1968 MB to 877 MB. **After the migration, going back to an older version is not supported.**

## Tables

| Table | Change | Columns |
|---|---|---|
| `frames` | Add `id`, drop `local_date` | `id` primary key, `path` unique and nullable, `ts`, `app_id`, `title`, `ocr_state`, `attempts`, `session_id` |
| `text_sessions` | None | The same 9 columns; `text` is now the only copy of the text |
| `session_frames` | Replaces `session_lines` | `session_id`, `first_line_no`, `frame_id`; primary key `(session_id, first_line_no)`, `WITHOUT ROWID` |

A screenshot's lines start at its `first_line_no` and end before the `first_line_no` of the next row in the same session. To find the screenshot of line k, take the row with the largest `first_line_no ≤ k`.

## Flow

```mermaid
sequenceDiagram
    participant A as App start
    participant M as Memory database<br/>(one connection,<br/>one queue)
    participant C as OCR and search

    A->>M: Open the database,<br/>create missing tables
    A->>M: Queue the maintenance<br/>task first (migration,<br/>VACUUM if needed)
    Note over A: Does not wait,<br/>shows the window
    C->>M: Reads and writes<br/>queue after it
    M->>M: session_lines still<br/>there: migrate in<br/>one transaction
    M->>M: Free pages over 1/4,<br/>free disk ≥ 2× the file:<br/>VACUUM, truncate WAL
    M-->>C: Done; runs the<br/>queued calls in order
```

What the maintenance task does at each start:

| `session_lines` still there | Free pages | Free disk ≥ 2× the database file | Action |
|---|---|---|---|
| Yes | — | — | Migrate in one transaction, then use the three rows below to decide on `VACUUM` |
| No | ≤ 1/4 | — | Nothing |
| No | > 1/4 | Yes | Run `VACUUM` to give the space back to the disk, then truncate the WAL |
| No | > 1/4 | No | Skip `VACUUM` and log it; check again at the next start |

If it stops in the middle:
- **The migration fails, or the process is killed during it**: the transaction rolls back and the old layout stays. The next start tries the migration again.
- **The migration is committed, then `VACUUM` fails or the process is killed**: the new layout stays; only the free pages are not given back. The next start skips the migration and tries `VACUUM` again.
- **Old lines whose screenshot is gone**: if a run of lines points to a screenshot that is not in `frames`, its `frame_id` is `NULL`. Search still finds these lines; only no screenshot is shown for them. The run cannot simply be dropped: its lines would then belong to the screenshot before it.

## Context

- **Repeated values take much disk**: on this computer the memory database is 1967 MB, and `session_lines` with its primary key index takes 1078 MB. Of this, the paths and times take 686 MB, and the text takes 176 MB, which `text_sessions.text` also holds. Its 5.75 million lines come from only 147,000 (session, screenshot) pairs: about 39 lines per screenshot.
- **One row per screenshot is enough**: on this computer, within a session the new lines of a screenshot always have consecutive line numbers, the line numbers have no gaps, and line k of the session text is the line with `line_no = k` (no exception for any of the three). So the first line number of each run is enough to find the screenshot of every line.
- **Not synced**: `session_lines` is not synced to other devices. `text_sessions`, which is synced, does not change, so the sync protocol does not change.
- **Links stay valid**: only three places delete rows from `frames`: purge by time, purge by app, and delete all. All three also delete the matching line records. Removing old screenshots deletes only the image files, not the `frames` rows.
- **Giving space back is slow and must not block the start**: deleting or rebuilding a table in SQLite only leaves free pages inside the file. Only `VACUUM` gives the space back to the file system, and it needs extra free disk and takes seconds. The app sets itself up before it shows the window, so the migration and `VACUUM` cannot run in that setup.

## Decision

We will **add an explicit primary key `id INTEGER PRIMARY KEY` to `frames` (`path` becomes `UNIQUE`) and drop `local_date`. `session_frames` replaces `session_lines`, and the text is kept only in `text_sessions.text`. An old database is changed in one transaction in the background after the start, and `VACUUM` runs when the free pages are over the limit**.

- **Whether `session_lines` exists decides the migration, not `user_version`**: older versions set `user_version` to 5 at every start, so the number cannot be trusted.
- **Rust finds the line with the keyword**: it reads the session text, splits it into lines, and then looks up the line number in `session_frames`. A session text is about 2 KB on average.
- **Purge by time deletes run by run**: before, it rebuilt the session text from `session_lines.text`, and that copy of the text is gone. Now it deletes the runs of lines from screenshots taken at or after the purge time, moves the later lines up, and updates `first_line_no` to match. It cannot just cut everything from the first deleted run: a screenshot that failed OCR is retried later, so its lines can sit after the lines of screenshots taken after it. This computer has 4 such runs. A run whose screenshot is no longer in `frames` has no known capture time, so it is kept.
- **The maintenance task is first in the connection's queue**: `MemoryDb` runs every call through one connection, one at a time. At start, the maintenance task is queued first and the open returns at once. The window is not blocked, and later OCR and search calls never see a half-done migration.
- **`path` can be NULL**: on this computer, the image files of 96% of the screenshots have expired and been deleted, but their paths are still in `frames`. A later PR will set `path` to `NULL` when old screenshots are removed, which saves about 26 MB more on this computer (about 13 MB each in the table and its unique index). SQLite cannot remove `NOT NULL` from a column without rebuilding the table. This migration rebuilds `frames` anyway, so allowing `NULL` now means no other migration later.
- **`frames` needs an explicit integer primary key**: `VACUUM` can renumber the hidden `rowid`. Only an explicit `INTEGER PRIMARY KEY` keeps each `frame_id` pointing to the same row after `VACUUM`.
- **`VACUUM` keeps no state**: nothing records whether it has run. Every start uses the same rule: free pages over 1/4 and enough free disk. The same rule covers the time right after the migration, a `VACUUM` that was stopped, and a large purge by the user.
- **Truncate the WAL after `VACUUM`**: in WAL mode, `VACUUM` first writes the new pages into the WAL. Without `wal_checkpoint(TRUNCATE)`, the WAL on this computer keeps taking 964 MB.
- **Query results do not change**: the frontend and the AI tools still get the screenshot's path and time.

## Alternatives

- **Only replace the path and time with `frame_id`, and keep one row per line**: fewer changes, but in the same command-line test the memory database only went down to 1216 MB, against 959 MB for this decision: about 260 MB more. Moving to this decision later would mean another migration and another `VACUUM` for users.
- **Keep `local_date` and the table name `session_lines` so that older versions still work**: this only helps after going back to an older version, and saves few changes. We decided not to support going back, and chose the smaller and clearer layout.
- **Store `frames.path` relative to the screenshot folder**: it saves only about 20 MB (about 10 MB each in the table and its unique index). And the screenshot folder can be changed in the settings: the screenshots taken before the change stay in the old folder, so a relative path would point to the wrong place.
- **Drop positions from the full-text index (FTS5 `detail='none'`)**: it would save about 330 MB more, but phrase search would break. With `LIKE` instead, a search for a common word takes 124 ms instead of 0.8 ms. Not done this time.

## Consequences

- **Benefits**: on this computer the memory database went from 1968 MB to 877 MB, 1091 MB smaller (−55%). `session_frames` takes only about 2 MB.
- **Costs and risks**:
  - At the first start after the update, OCR and screen text search wait for the maintenance task. On this computer, opening the memory database returned after 4 ms, the migration took 11.1 s, `VACUUM` took 4.4 s, and the first call after the open returned after 15.6 s. These numbers are from a development build (main crate O1, dependencies O2). A release build (`opt-level = "z"`) was not measured. Slower disks take longer.
  - The maintenance task needs extra disk for a short time: on this computer the peak was 2979 MB (main file 2014 MB + WAL 964 MB), about 1.5 times the original file. The WAL is truncated at the end of the maintenance; it measured 0 MB.
  - The screen text search tool in AI chat uses its own read-only connection, outside the queue. If the user asks a question in the few seconds before the migration commits, the tool can fail once.

## Data, compatibility, security, and privacy

- **Migration, no way back**: the layout changes in one transaction, all at once. After it, the path and time are kept only in `frames` and the text only in `text_sessions`, and the change cannot be undone. On this computer the migration wrote 147,141 runs. A check of all 5,759,707 lines found 0 lines whose screenshot differs from the old table, and 0 changed session texts.
- **Going back to an older version (0.12.x) is not supported**: with the migrated database, an older version fails to register screenshots and to read the screenshots waiting for OCR, because `frames.local_date` is gone, so the screen memory stops recording. Sessions from before the update also show no screenshot. The release notes must warn users not to go back to an older version after the update.
- **Privacy**: the maintenance logs give only row counts, times, and file sizes. They never contain screen text or file paths.
