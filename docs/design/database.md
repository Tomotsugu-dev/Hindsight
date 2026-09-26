# SQLite Database Schema

Hindsight keeps two SQLite files under the data directory (`~/Library/Application Support/Hindsight/` on macOS):

- `hindsight.<uid>.sqlite` — the main database: activities, classification, sync, devices, settings. Covered below.
- `hindsight-memory.<uid>.sqlite` — screen memory: screenshot frames, OCR text sessions, FTS index, chat history. `frames`, `text_sessions` and `session_lines` are covered under *Memory database* below.

Each table below sits under the heading of the file it lives in.

# Main database (`hindsight.<uid>.sqlite`)

Schema changes are applied by `src-tauri/src/storage/migrations.rs` (append-only, tracked in `schema_version`). Timestamps are RFC3339 strings; `updated_at` columns are UTC and drive last-write-wins merging during sync; `deleted_at` columns are soft-delete tombstones (ordinary writes never physically delete rows, so the deletion can be synced; app deletion via `purge_with_data` is the one exception).

## activities

One row per focus session: the user had `process_name` in the foreground from `started_at` to `ended_at`. This is the only table that grows with time (~1,500 rows/day); every query against it filters by date or aggregates in SQL.

| Field | Type | Constraints | Description |
|---|---|---|---|
| id | INTEGER | Primary key, autoincrement | |
| started_at | TEXT | NOT NULL | Start time, RFC3339 in the capturing device's local offset. Compare with `datetime()`, not as plain strings, since offsets differ across devices |
| ended_at | TEXT | NOT NULL | End time, same format as `started_at` |
| duration_secs | INTEGER | NOT NULL | Length of the session in seconds |
| local_date | TEXT | NOT NULL | Calendar date on the capturing device, `YYYY-MM-DD`. All report windows filter on this |
| local_hour | INTEGER | NOT NULL | Hour of day on the capturing device, 0–23 |
| process_name | TEXT | NOT NULL | Process name as reported by the OS. Resolves to a logical app through `app_group_members` |
| window_title | TEXT | | Foreground window title, if captured |
| category_id | TEXT | NOT NULL | **Deprecated — always `'other'`.** The real category lives on `app_groups.category_id`. Kept only because older peers still expect the field in the sync payload; do not read it |
| screenshot_path | TEXT | | Path of the screenshot taken during this session, if any |
| image_hash | INTEGER | | **Unused legacy column** from the first schema; never written by any code path |
| device_id | TEXT | NOT NULL, DEFAULT `'local'` | Device that captured the row |
| remote_id | TEXT | | The row's `id` on the device that captured it. Local rows get their own `id` via trigger. `(device_id, remote_id)` is the de-duplication key for sync |
| updated_at | TEXT | NOT NULL, DEFAULT epoch | Last-write-wins timestamp, UTC |
| origin | TEXT | NOT NULL, DEFAULT `'local'` | `'local'` if captured here, `'remote'` if pulled from another device |
| excluded | INTEGER | NOT NULL, DEFAULT 0 | 1 when an ignore rule (process + title keywords) matches, and stats, reports, exports and AI summaries skip the row. The column is not in the sync payload — each device tags rows with its own rules, including the ones it pulls — but **the row itself is still uploaded**. See docs/design/data-controls.md |
| url_host | TEXT | | Browser sessions only: the site's domain (never the full URL). NULL for non-browser sessions or when the "record browser domains" setting is off. Added in v0.8.20 |

Indexes: `(local_date)`, `(local_date, local_hour)`, `(process_name)`, `(device_id)`, `(device_id, remote_id)` UNIQUE.
Foreign keys: none.

## categories

User-visible categories ("Work", "Browsing", …). Bounded by how many the user creates; a few dozen at most.

| Field | Type | Constraints | Description |
|---|---|---|---|
| id | TEXT | Primary key | Short word for built-ins (`work`, `code`, `browse`, `other`, `hidden`), UUID for user-created ones |
| name | TEXT | NOT NULL | Display name |
| color | TEXT | NOT NULL | Hex color `#rrggbb` |
| builtin | INTEGER | NOT NULL, DEFAULT 0 | System category: belongs to the app, so it cannot be deleted, dragged, or filed under a super-category. `hidden` is the only one; the seeded defaults are ordinary user categories. `other` is undeletable too, but by an id check rather than this flag |
| icon | TEXT | NOT NULL, DEFAULT `'Tag'` | Icon id, mapped to a lucide-react icon by the frontend |
| sort_order | INTEGER | NOT NULL, DEFAULT 0 | Display order; rewritten when the user drags rows |
| updated_at | TEXT | NOT NULL, DEFAULT epoch | Last-write-wins timestamp, UTC |
| deleted_at | TEXT | | Soft-delete tombstone |
| super_category_id | TEXT | | Parent super-category, or NULL when ungrouped. No foreign key |

Indexes: none beyond the primary key.
Foreign keys: none.

## app_groups

One logical app as the user thinks of it ("Chrome"), regardless of how many process names it runs under. Classification lives here.

| Field | Type | Constraints | Description |
|---|---|---|---|
| id | TEXT | Primary key | Equals the first `process_name` seen for the app, so two devices backfill the same id independently. Unchanged by merges |
| display_name | TEXT | NOT NULL | Name shown in the UI |
| category_id | TEXT | | **Source of truth for classification.** NULL = unclassified. No foreign key on purpose: sync may pull a peer's groups file before its categories file, and a foreign key would reject those rows. `assign_category` validates the id in code instead |
| updated_at | TEXT | NOT NULL, DEFAULT epoch | Last-write-wins timestamp, UTC |
| deleted_at | TEXT | | Soft-delete tombstone |

Indexes: none beyond the primary key.
Foreign keys: none.

## app_group_members

Maps each process name to its group. Every process name ever seen has an active row here.

| Field | Type | Constraints | Description |
|---|---|---|---|
| process_name | TEXT | Primary key | Process name as reported by the OS |
| group_id | TEXT | NOT NULL, FK → `app_groups(id)` | The group this process belongs to. Initially equals `process_name`; changed by merge/unmerge |
| updated_at | TEXT | NOT NULL, DEFAULT epoch | Last-write-wins timestamp, UTC |
| deleted_at | TEXT | | Soft-delete tombstone |

Indexes: `(group_id)`.

## sync_outbox

Queue of local changes waiting to be pushed to the cloud. Each business write enqueues a row in the same transaction, so a persisted change is always sync-reachable. Rows are deleted after a successful push.

| Field | Type | Constraints | Description |
|---|---|---|---|
| id | INTEGER | Primary key, autoincrement | |
| op | TEXT | NOT NULL | Always `'upsert'`, and nothing reads it: push only uses `entity` and `payload`. Deletions are upserts carrying `deletedAt` |
| entity | TEXT | NOT NULL | Which cloud file the next push rewrites: `activity`, `category`, `app_group`, `app_group_member`, `device`, `app_icon`. A row with any other entity, such as `app_category` or `process_path` left behind by older versions, is logged and deleted |
| entity_pk | TEXT | NOT NULL | Primary key of the changed row |
| payload | TEXT | NOT NULL | JSON. Push reads it only for `activity`, to take `localDate` and pick the day's file; for every other entity it is ignored. It is never sent to other devices — push rebuilds each file from the tables |
| created_at | TEXT | NOT NULL | |
| attempts | INTEGER | NOT NULL, DEFAULT 0 | Failed push attempts so far |
| last_error | TEXT | | Message from the last failed attempt |
| next_retry_at | TEXT | NOT NULL | Earliest time the row is eligible for the next push |

Indexes: `(next_retry_at)`.
Foreign keys: none.

## sync_cursor

Where sync has got to: one row per pull stream, and one per optional dataset for push. The column name only fits the pull rows.

| Field | Type | Constraints | Description |
|---|---|---|---|
| entity | TEXT | Primary key | The row's name; see the table below |
| last_pulled_at | TEXT | NOT NULL, DEFAULT epoch | See below |

Each backend keeps its own rows, so one backend never reads another's progress, and switching backends clears nothing (ADR-0011 §3):

| Rows | Drive | A WebDAV server |
|---|---|---|
| Pull cursors | `drive_files`, `pull.ai_summaries`, `pull.chat`, `pull.memory` | `webdav.<host>.pull.core`, `webdav.<host>.pull.ai_summaries`, … |
| Push fingerprints | `push.ai_summaries`, `push.chat`, `push.memory` | `webdav.<host>.push.ai_summaries`, … |
| Bookmarks | — | `webdav.<host>.peer.<device id>` |
| Pending changes | — | `webdav.<host>.pending` |

Drive's names are the ones it had before WebDAV. Existing databases hold them, so they must not change. `<host>` is the server's host, with the port only when it is not the default, e.g. `dav.jianguoyun.com`.

The pull cursors are one per stream: the core one (`drive_files` on Drive) covers activities, categories, app groups, members, icons, device meta and tombstones, and each other `pull.*` row covers one optional dataset (ADR-0006). For its own kind of file, a cursor says that every cloud file modified at or before this time (UTC, as the backend reports it: Drive's `modifiedTime`, or a WebDAV manifest's server time) has been merged or deliberately skipped. A stream runs only while its dataset's switch is on; while it is off its cursor stays put, so turning the switch on resumes from there instead of re-merging everything.

Each round lists files once, from the earliest cursor among the streams that run, and a stream's cursor may move only across the files it has handled, stopping strictly before the first file of its own kind that failed: a failed file at or before the cursor would never be listed again.

The `push.*` rows record what each optional dataset looked like at its last upload. Before each push the value is computed again from the tables; unchanged means nothing is uploaded.

- `push.ai_summaries`: `<max generated_at>:<row count>` of `ai_summaries`.
- `push.chat`: `<max updated_ts>:<row count>` of `chat_conversations`, then `|`, then `<max created_ts>:<row count>` of `chat_messages`. Both tables are in the memory database.
- `push.memory`: the latest `ended_ts` among this device's own rows in `text_sessions` (memory database). It is also where the next upload starts: only the days with a session that ended after it are uploaded again.

The bookmark and pending rows exist only on WebDAV, and the engine never reads them (ADR-0009):

- `peer.<device id>`: that peer's bookmarks, as JSON. The keys are dataset names (`core`, `ai_summaries`, `chat`, `memory`), and each value is `{processed, manifest_time}`: the peer's push count this dataset has merged up to, and the server time of the manifest version it was recorded at. A missing key means that dataset has not synced with this peer yet.
- `pending`: this device's files that were uploaded or deleted but not yet written into its own manifest, as JSON `{uploaded, removed}`, with paths relative to the device directory. The row is deleted once the manifest is written.

On WebDAV the listing does not go by time: the engine passes every stream's cursor, and the bookmarks decide which files are reported.

A missing row reads as the epoch, meaning "never": a row appears the first time that cursor or fingerprint is written. So a new install, or a backend used for the first time, pulls every cloud file, a dataset switched on for the first time pulls its whole history, and the first push of a dataset uploads it. Nothing sets a cursor back.

Indexes: none beyond the primary key.
Foreign keys: none.

## devices

Every device that has ever synced into this account, including this one.

| Field | Type | Constraints | Description |
|---|---|---|---|
| device_id | TEXT | Primary key | UUID assigned on first run |
| display_name | TEXT | NOT NULL | Name shown in the UI |
| color | TEXT | NOT NULL, DEFAULT `'#60a5fa'` | Avatar color |
| icon | TEXT | NOT NULL, DEFAULT `'Monitor'` | Icon id |
| os | TEXT | | `'win'`, `'mac'` or `'linux'`; NULL if the peer never reported it |
| last_seen_at | TEXT | | Latest `ended_at` among the device's activities |
| is_self | INTEGER | NOT NULL, DEFAULT 0 | 1 for the current machine |
| updated_at | TEXT | NOT NULL, DEFAULT epoch | Last-write-wins timestamp, UTC |
| deleted_at | TEXT | | Soft-delete tombstone, set when the user forgets a remote device |

Indexes: none beyond the primary key.
Foreign keys: none.

## auth_state

Sign-in state for this device: which backend and which Drive account this database belongs to (ADR-0011), and the credentials. Exactly one row, pinned by `CHECK (id = 1)` and inserted empty by the migration, so the row always exists — signed out is NULL credential columns, never a missing row.

| Field | Type | Constraints | Description |
|---|---|---|---|
| id | INTEGER | Primary key, `CHECK (id = 1)` | Always 1 |
| uid | TEXT | | Google account id |
| email | TEXT | | Account email, shown in the UI |
| refresh_token_enc | BLOB | | Long-lived refresh token, AES-GCM encrypted under a key derived from the machine id and the user's home directory |
| access_token | TEXT | | Short-lived token sent with every Drive call |
| expires_at | TEXT | | When `access_token` stops working, UTC |
| backend | TEXT | `CHECK (backend IN ('drive', 'webdav'))` | The kind of backend this database syncs with now; NULL if it has never synced (v39) |
| drive_account | TEXT | | This database's Drive account, a Google uid. Kept after signing out; databases older than v39 get it from their file name at startup (v39) |
| webdav_url | TEXT | | The WebDAV server currently connected (v39) |
| webdav_user | TEXT | | The user name on that server (v39) |
| webdav_password_enc | BLOB | | The WebDAV password, encrypted like `refresh_token_enc` (v39) |

A caller counts as signed in to Google only when `uid`, `refresh_token_enc`, `access_token` and `expires_at` are all non-NULL. Any partial state reads as signed out, so nothing downstream has to handle "has an account id but no token".

Signing out of Google NULLs `uid`, `email` and the three token columns and keeps the row; `backend` and `drive_account` stay, so the next sign-in can tell whether it is the same account. Renewing a token writes only `access_token` and `expires_at`.

At startup the app builds WebDAV when `backend` is `webdav` and `webdav_url` is set, and Drive otherwise. WebDAV reads `webdav_user` and `webdav_password_enc` at the start of every round, and counts as signed in only when both are non-NULL. A password that does not decrypt on this machine fails the round as an invalid credential, so the Devices page asks for it again.

Never synced, and it could not be: the encryption key is derived from the machine, so the blob is meaningless anywhere else.

Indexes: none beyond the primary key.
Foreign keys: none.

## webdav_accounts

Which account this database has used on each WebDAV server, one row per server (ADR-0011 §4). `auth_state` holds only the server in use now; this table keeps the others too. When the user connects to a server again, the app normalizes the address and user name they entered and compares them with this row to tell whether it is the same account. A backend switch to WebDAV writes the row. Never synced.

| Field | Type | Constraints | Description |
|---|---|---|---|
| host | TEXT | Primary key | The server's host, with the port only when it is not the default, e.g. `dav.jianguoyun.com` (v40) |
| url | TEXT | NOT NULL | The server address, as the user last entered it (v40) |
| user | TEXT | NOT NULL | The user name, as the user last entered it (v40) |

Indexes: none beyond the primary key.
Foreign keys: none.

## settings_store

Every setting on this device as one JSON document: the Settings page, the Google OAuth client entered on the Devices page, AI configuration and ignore rules. Exactly one row, pinned by `CHECK (id = 1)` and inserted as `'{}'` by the v7 migration, so the row always exists.

| Field | Type | Constraints | Description |
|---|---|---|---|
| id | INTEGER | Primary key, `CHECK (id = 1)` | Always 1 |
| data | TEXT | NOT NULL | The `Settings` struct in `src-tauri/src/repo/settings.rs`, serialized as camelCase JSON. The struct is the field list |

A single document instead of a column per setting, so a new setting needs no migration: `Settings` is `#[serde(default)]`, and a key missing from an older document reads as its default. The cost is that SQL cannot address one setting. To change an existing value for every user, a migration edits the JSON in place: v17 appends `/password` to `privacyUrlKeywords`, v23 resets `screenshotEnabled` to false.

At runtime only `repo::settings::load` and `save` query the table. `save` replaces the whole document, so callers load, change and save. `load` fills in a few defaults (an empty screenshot or model directory, legacy AI fields) and writes the result back. If the JSON fails to parse, `load` continues with defaults in memory and does not write them back; it first copies the raw document to `settings_store.corrupt.json` in the data directory, with `apiKey` and `googleClientSecret` masked.

Secrets are plain text here: the AI provider `apiKey` and `googleClientSecret`. Unlike `auth_state.refresh_token_enc`, nothing in this table is encrypted.

Never synced. Each device keeps its own settings, including the three switches that choose which optional datasets sync.

Indexes: none beyond the primary key.
Foreign keys: none.

## process_paths

Executable path per process name, recorded by capture so an icon can be extracted from the executable. That is its only use; the frontend never sees a path. Local only: it was synced until ADR-0003 took it out, and rows a peer wrote back then may remain until the app is used again here.

| Field | Type | Constraints | Description |
|---|---|---|---|
| process_name | TEXT | Primary key | Process name as reported by the OS |
| exe_path | TEXT | NOT NULL | Absolute path of the executable on this device |
| seen_at | TEXT | NOT NULL | When the path was recorded |
| updated_at | TEXT | NOT NULL, DEFAULT epoch | Left over from sync; nothing reads it |

No `deleted_at`: the table cannot tombstone. App deletion physically deletes the row.
Indexes: none beyond the primary key.
Foreign keys: none.

Everything that touches the table. Check each one before changing its shape:

| Kind | Function | What it does |
|---|---|---|
| Write | `repo::process_paths::upsert`, called from `capture::service` | Records this device's path on every capture tick that reports one, not only when focus changes |
| Read | `commands::icons::resolve_icon_png`, behind the `get_app_icon` and `get_app_icon_data_url` commands | Last of three fallbacks: extracts the icon from the executable when neither the file cache nor a synced icon has it |
| Delete | `commands::storage::purge_activities` | Clears the whole table |
| Delete | `repo::app_groups::purge_with_data` | Deletes the rows of the app being deleted |

## app_icons

Icon bitmap per process name. Synced per device as `device.<id>.icons.json` (`app_icon` entity), so a peer that never ran the app still gets its icon.

| Field | Type | Constraints | Description |
|---|---|---|---|
| process_name | TEXT | Primary key | Process name as reported by the OS |
| icon_png | BLOB | NOT NULL | PNG bytes |
| updated_at | TEXT | NOT NULL, DEFAULT epoch | Last-write-wins timestamp, UTC |
| deleted_at | TEXT | | Soft-delete tombstone. App deletion does not use it: it physically deletes the row |

Indexes: none beyond the primary key.
Foreign keys: none.

## screenshot_dedup_map (Dead)

Added in v35 to record which screenshot frames the MobileNet-embedding dedup folded into a representative frame. **No code writes it any more**: the dedup was removed with MobileNet, and the only remaining statements are `DELETE`s (app deletion, purge activities, clear screenshots). Rows on older installs are read-only residue; `docs/design/screen-memory.md` plans to migrate them into an `attach_map` table.

| Field | Type | Constraints | Description |
|---|---|---|---|
| member_path | TEXT | Primary key | Path of the frame that was folded away |
| rep_path | TEXT | NOT NULL | Path of the representative frame it was folded into |
| local_date | TEXT | NOT NULL | Calendar date of the frames |
| created_at | TEXT | NOT NULL | |

Indexes: `(rep_path)`, `(local_date)`.
Foreign keys: none.

# Memory database (`hindsight-memory.<uid>.sqlite`)

Created by `init_schema` in `src-tauri/src/memory/mod.rs`: one idempotent `CREATE ... IF NOT EXISTS` batch plus per-column `ALTER TABLE` back-fills, with `PRAGMA user_version` (currently 5) as a generation marker rather than a migration ladder. Runs in WAL mode. Paths in this database are relative to the screenshot root, the same form as `activities.screenshot_path`; `app_id` columns hold the process name, the same key as `activities.process_name`.

The three tables of the OCR pipeline are documented, in pipeline order: `frames` (screenshots waiting for OCR) → `text_sessions` (the OCR text, folded and searchable) → `session_lines` (that text line by line). `frame_insights`, the chat tables and `scheduled_ocr_marks` are not covered yet.

## frames

The screenshot ledger: one row per screenshot file on disk. The capture side registers a row right after the file lands (`INSERT OR IGNORE`, so re-registering the same path is a no-op); the OCR worker takes rows by `ocr_state`, reads the text, and folds the frame into a `text_sessions` row. Local only, never synced.

| Field | Type | Constraints | Description |
|---|---|---|---|
| path | TEXT | Primary key | Screenshot path relative to the screenshot root, the same value as `activities.screenshot_path` |
| ts | TEXT | NOT NULL | Capture time, RFC3339 |
| local_date | TEXT | NOT NULL | Calendar date on this device, `YYYY-MM-DD` |
| app_id | TEXT | | Process name of the foreground app at capture time. Nullable in the schema, never NULL in practice |
| title | TEXT | | Foreground window title |
| ocr_state | INTEGER | NOT NULL, DEFAULT 0 | 0 pending, 1 done, 2 failed |
| attempts | INTEGER | NOT NULL, DEFAULT 0 | Failed OCR attempts so far; the worker retries up to a budget |
| session_id | INTEGER | | The `text_sessions.id` this frame was folded into. NULL exactly while the frame is pending or failed. No foreign key |

Indexes: `(ocr_state, ts)`, which is how the worker picks the oldest pending frame.
Foreign keys: none.

## text_sessions

The unit of screen-memory search. Consecutive screenshots of the same app and window title are folded into one session; `text` holds their OCR output with duplicate lines removed, and full-text search runs over that column. By far the largest table in either file: tens of thousands of rows, each carrying a page of text.

Synced only when screen-memory sync is enabled, as `device.<id>.memory.<date>.ndjson`; each device pushes only the sessions it produced (`origin_device IS NULL`) and stores what it pulls with `origin_device` set to the sender.

| Field | Type | Constraints | Description |
|---|---|---|---|
| id | INTEGER | Primary key | Local rowid; differs across devices, which is why `guid` exists |
| local_date | TEXT | NOT NULL | Calendar date on the producing device, `YYYY-MM-DD` |
| started_ts | TEXT | NOT NULL | Capture time of the first frame, RFC3339 |
| ended_ts | TEXT | NOT NULL | Capture time of the last frame, RFC3339 |
| app_id | TEXT | | Process name of the foreground app |
| title | TEXT | | Window title the session was folded under |
| text | TEXT | NOT NULL, DEFAULT `''` | Materialised concatenation of the session's `session_lines`; the FTS index is built on this column |
| guid | TEXT | UNIQUE index | Global id for cross-device merging. Added later and back-filled with random hex, so it is never NULL in practice |
| origin_device | TEXT | | NULL when produced on this device; otherwise the id of the device it was pulled from. Added later |

Indexes: `(guid)` UNIQUE.
Foreign keys: none.

Full-text search: `text_sessions_fts` is an FTS5 external-content table over `text` with the trigram tokenizer, so CJK substrings match without word segmentation. Three triggers keep it in step (`text_sessions_ai` after insert, `text_sessions_au` after update of `text`, `text_sessions_ad` after delete), which is why callers only ever touch `text_sessions` itself. FTS5 keeps its own shadow tables (`text_sessions_fts_data`, `_idx`, `_config`, `_docsize`); they are not part of the schema you write to.

## session_lines

One row per unique OCR line within a text session, together with the first screenshot the line appeared in, so an evidence card can open the exact frame rather than the whole session. Local only, never synced. Roughly forty lines per session, so this is the table with the most rows in either file.

| Field | Type | Constraints | Description |
|---|---|---|---|
| session_id | INTEGER | NOT NULL, part of primary key | The owning `text_sessions.id`. **No foreign key**, so deleting a session does not cascade; callers must delete the lines first or they become orphans |
| line_no | INTEGER | NOT NULL, part of primary key | Position of the line within the session |
| text | TEXT | NOT NULL | The line as OCR read it |
| first_path | TEXT | NOT NULL | Screenshot path where the line first appeared |
| first_ts | TEXT | NOT NULL | Capture time of that screenshot, RFC3339 |

Indexes: primary key `(session_id, line_no)`.
Foreign keys: none.

