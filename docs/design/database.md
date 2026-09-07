# SQLite Database Schema

Hindsight keeps two SQLite files under the data directory (`~/Library/Application Support/Hindsight/` on macOS):

- `hindsight.<uid>.sqlite` — the main database: activities, classification, sync, devices. Covered below.
- `hindsight-memory.<uid>.sqlite` — screen memory: screenshot frames, OCR text sessions, FTS index, chat history. Not covered here yet.

Schema changes are applied by `src-tauri/src/storage/migrations.rs` (append-only, tracked in `schema_version`). Timestamps are RFC3339 strings; `updated_at` columns are UTC and drive last-write-wins merging during sync; `deleted_at` columns are soft-delete tombstones (rows are never physically deleted, so the deletion can be synced).

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
| excluded | INTEGER | NOT NULL, DEFAULT 0 | 1 when an ignore rule (process + title keywords) matches. Excluded from stats and reports; not synced |
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
| builtin | INTEGER | NOT NULL, DEFAULT 0 | 1 for built-in categories, which cannot be deleted. `other` is refused too even though it is seeded with 0: reports bucket unclassified time into it |
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
| category_id | TEXT | | **Source of truth for classification.** NULL = unclassified. No foreign key; `assign_category` validates the id in code |
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

## app_categories (Deprecated)

**Legacy mirror** of `process_name → category_id`, from before groups existed. Reads stopped using it in **v0.7.0**, when `categories::list` and `list_unclassified` switched to the group chain. Local writes stop in **<next release after v0.8.22>** (branch `fix/categories-mirror-retire`; replace with the version number when it ships): sync push now derives the equivalent file from `app_group_members ⋈ app_groups`, and pull still stores what older peers send so the `app_category` sync entity stays compatible. Nothing reads it.

| Field | Type | Constraints | Description |
|---|---|---|---|
| process_name | TEXT | Primary key | |
| category_id | TEXT | NOT NULL, FK → `categories(id)` | The only foreign key that validates a category id — which is why `app_groups` needed explicit validation once this table stopped being written |
| updated_at | TEXT | NOT NULL, DEFAULT epoch | |
| deleted_at | TEXT | | |

Indexes: none beyond the primary key.

## sync_outbox

Queue of local changes waiting to be pushed to the cloud. Each business write enqueues a row in the same transaction, so a persisted change is always sync-reachable. Rows are deleted after a successful push.

| Field | Type | Constraints | Description |
|---|---|---|---|
| id | INTEGER | Primary key, autoincrement | |
| op | TEXT | NOT NULL | `'upsert'`. `'delete'` is defined but unused — deletions are upserts carrying `deletedAt` |
| entity | TEXT | NOT NULL | Kind of row: `activity`, `category`, `app_group`, `app_group_member`, `app_category`, `process_path`, `device`, `app_icon` |
| entity_pk | TEXT | NOT NULL | Primary key of the changed row |
| payload | TEXT | NOT NULL | JSON snapshot of the whole row (not a diff), camelCase keys — what the receiving device applies with last-write-wins |
| created_at | TEXT | NOT NULL | |
| attempts | INTEGER | NOT NULL, DEFAULT 0 | Failed push attempts so far |
| last_error | TEXT | | Message from the last failed attempt |
| next_retry_at | TEXT | NOT NULL | Earliest time the row is eligible for the next push |

Indexes: `(next_retry_at)`.
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
