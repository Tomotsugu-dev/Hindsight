# ADR-0011 · Switching sync backends and WebDAV accounts

- **Date**: 2026-09-23
- **Status**: **Accepted**
- **Related**: Supersedes ADR-0007 §2 ("switching backends is switching accounts") and its guidance on new databases, passwords, and sync keys · ADR-0002 (local key) · ADR-0005 (whole-file rewrites) · ADR-0006 (one pull cursor per dataset) · ADR-0009 (WebDAV bookmarks)

## Context

The existing rule is "one local database per account." When a user switches Google accounts, the app opens a different database so data from the two accounts never mix.

With WebDAV, switching backends does not necessarily mean switching accounts. A user may switch from Google Drive to WebDAV, then switch back. The old design treated these as the same operation and created an empty database when switching to a backend the user had not used before. As a result, history and screen memory stayed in the old database and became unavailable after the switch, while another database took up additional disk space.

ADR-0007 also left two questions unanswered: how to identify a WebDAV account and where to store its password.

- The WebDAV password cannot be stored in ordinary settings because `get_settings` returns the entire settings object to the frontend.
- The local key defined in ADR-0002 encrypts credentials stored on this device; it is not a cloud sync key. Cloud files are not encrypted with it.

## Decision

### 1. Distinguish switching backends from switching accounts

In this ADR, a "backend" means Drive or a specific WebDAV server. Different WebDAV servers, such as Nutstore and Nextcloud, count as different backends and are distinguished by their normalized hostnames (see §5).

The two kinds of switch are handled differently:

- **Switching backends**: Keep using the current local database. Each backend has its own sync progress. Sync from the beginning the first time a backend is used; when returning to it, resume from where it was left.
- **Switching accounts on the same backend**: Do not keep using the current database. Create or open a different one so data from the two accounts cannot mix.

Examples:

| Change | Local database |
|---|---|
| Drive account A → Nutstore account a | Keep the current database |
| Nutstore account a → Nextcloud account n | Keep the current database |
| Nutstore account a → Drive account A | Keep the current database |
| Drive account A → Drive account B | Create or open B's database |
| Nutstore account a → Nutstore account b | Create or open b's database |

Each local database records at most one account for each backend. A user can therefore switch between backends and keep using the same database. The database changes only when the account changes on the same backend.

For example, a user switches from Drive account A to WebDAV, then signs in to Drive account B. Because B differs from the Drive account A recorded in the current database, the app must create or open another database for B instead of reusing A's.

When an anonymous database (`hindsight.sqlite`) connects to any backend for the first time, keep using it and claim it for that account, as with the first Google sign-in. Record the account, then rename the database on the next startup to use the account-specific filename: `hindsight.<uid>.sqlite` for Drive or `hindsight.<account-hash>.sqlite` for WebDAV (see §5). Without the rename, the app cannot find this database by account if the user later switches accounts on the same backend and then switches back.

### 2. Perform backend switches in one transaction

When switching from one backend to another, perform all three steps in a single transaction on the current local database:

1. Save the new backend and its credentials, and record the account for that backend.
2. Refill the outbox with one pending item for each of these files:
	- Activities are stored by day in `activities.<date>.ndjson`; add one item for each date with activity on this device.
	- Add one item for each of the five whole-table files: categories (`categories.json`), device metadata (`meta.json`), app icons (`icons.json`), app groups (`app_groups.json`), and group members (`app_group_members.json`).

	Push rewrites each file from its table (ADR-0005), so one pending item per file is enough.
3. Delete the old backend's credentials: the Google token or the WebDAV password.

All files in step 2 must be uploaded again on every switch. Their push progress is not tracked per backend; they share one outbox queue. Each local write adds a pending item, and whichever backend receives the push removes that item. When switching back to a previously used backend, the other backend has already removed the pending items, so the app cannot tell which files the returning backend is missing.

The three optional datasets (AI summaries, chat history, and screen memory) do not use the outbox and are not re-uploaded in step 2. On each push, the app computes their push fingerprints directly from the tables and compares them with the per-backend fingerprints described in §3. It uploads a dataset only when its fingerprint differs, then stores the new fingerprint.

As a result, the first push to a backend uploads each dataset in full because no fingerprint has been recorded for it yet. When returning to a previously used backend, only data changed since the last time it was used is uploaded. AI summaries and chat history each use a single file, so any change causes the whole file to be uploaded again.

After the transaction commits, the regular sync process completes the remaining work:

- Push uploads the data already on this device to the new backend.
- Pull merges data according to that backend's own progress: from the beginning on first use, or from where it was left when returning to a previously used backend.

Data is merged using upserts, so processing it more than once does not create duplicate records.

These three steps must be in the same transaction. If the app crashes after saving the new backend but before refilling the outbox, this device's history may never be uploaded to the new backend. All of this state is stored in the main database, so one transaction can ensure the steps either all complete or all roll back.

### 3. Keep separate sync progress for each backend

`sync_cursor` stores sync progress separately for each backend:

- Drive continues to use its existing rows: four pull cursors (`drive_files`, `pull.*`) and three push fingerprints (`push.*`).
- Each WebDAV server uses a set of rows prefixed with `webdav.<host>.`: pull cursors (`pull.*`), push fingerprints (`push.*`), bookmarks (`peer.*`), pending changes (`pending`), and the account for that server (`account`; see §4).

For example, a local database that has used Nutstore might have these rows in `sync_cursor`. `entity` is the row name, and `last_pulled_at` stores its value. The column name is historical; WebDAV rows may store values other than timestamps:

| `entity` | `last_pulled_at` |
|---|---|
| `webdav.dav.jianguoyun.com.account` | `webdav-3f2a…` (account hash for this server) |
| `webdav.dav.jianguoyun.com.pull.core` | `2026-10-01T10:00:00Z` (pull cursor for the core dataset) |
| `webdav.dav.jianguoyun.com.peer.<device-id>` | The bookmark for that peer device, as JSON |

Do not clear any backend's sync progress when switching backends.

Different backends must not share cursors because cursor timestamps come from their respective servers. Sharing cursors would cause the following problems:

- **Switching from Drive to WebDAV**: Comparing a Drive cursor such as `11:05:01` with the manifest timestamp on Nutstore could make a version published by a peer at `10:55` appear to have already been processed. The app would update the bookmark without downloading the file, and data for those earlier dates would never be discovered later.
- **Rolling back to an older version**: Older versions recognize only Drive rows. If WebDAV writes a Nutstore timestamp into those rows, an older version may use it after the user signs back in to Google to ask Drive "which files changed after this time?" Files uploaded to Drive by other devices in the meantime could be skipped.
- **Switching between WebDAV servers**: Their timestamps differ, and push counts in their bookmarks are independent; neither can be compared across servers.

Keeping progress separate lets older versions resume from the point at which Drive was left and catch up on intervening changes. When returning to a previously used backend, the app also avoids downloading data it has already merged.

### 4. Account information to store

Add five columns to `auth_state`:

- `backend`: The kind of backend this database currently uses, either `drive` or `webdav`; empty if it has never synced.
- `drive_account`: This database's Drive account, identified by its Google uid.
- `webdav_url`, `webdav_user`: The current WebDAV server and username.
- `webdav_password_enc`: The password for the current WebDAV account.

The account used by this local database on each WebDAV server is recorded in that server's `webdav.<host>.account` row in `sync_cursor`. Its value is the account hash defined in §5.

Switching backends does not change the account identities recorded in the local database. If the same WebDAV account uses a different URL or username spelling, update `webdav_url` and `webdav_user`; the account hash stays the same. When switching to a different account, its local database records its own account information.

Signing out deletes only the credentials (the Google token or WebDAV password); retain the other account information. The app can then determine on the next connection whether it is the same account.

Encrypt the WebDAV password with the local key from ADR-0002, using the same protection as for the Google refresh token.

### 5. WebDAV account hashes and server host prefixes

Unlike Google, WebDAV servers do not provide a uid for each account. The app therefore computes an account hash locally to recognize the same account. The server does not know this value; it is used in two places on this device:

- The filename of a local database created for or claimed by the account: `hindsight.<account-hash>.sqlite`.
- The account's `webdav.<host>.account` row in `sync_cursor`, which is used on the next connection to determine whether it is the same account (see §4).

The account hash has this format:

```text
webdav-<first 16 hexadecimal characters of SHA-256>
```

The hash input is the normalized URL, followed by a newline and the normalized username. Normalize them as follows:

- URL: lowercase the scheme and hostname, remove the default port, and remove the trailing `/`.
- Username: trim surrounding whitespace and convert to lowercase.

For example, these two connections produce the same account hash:

| URL | Username |
|---|---|
| `HTTPS://DAV.jianguoyun.com:443/dav/` | `You@Example.com` |
| `https://dav.jianguoyun.com/dav` | `you@example.com` |

The `<host>` in row names from §3 comes from the normalized URL. Keep non-default ports, for example `dav.jianguoyun.com` and `cloud.example.com:8443`.

The account-hash and server-host-prefix functions must have tests with fixed inputs and outputs. If either rule changes unintentionally, existing users could get a new account hash after restarting and be mistaken for a new account, opening an empty database. The app could also fail to find their existing sync progress and download data again.

## Alternatives

- **Create a new database on every backend switch** (the former ADR-0007 approach): Data from different backends stays separate, but after switching the user cannot see the history and screen memory in the old database, and the extra database takes up disk space.
- **Build the account hash from the username alone, or from the username and password**: With only the username, accounts with the same name on different servers (for example, `admin`) get the same hash and open the same local database. Including the password means a password change leads to an empty database; its hash would also appear in the filename and could be targeted for offline cracking.
- **Share one set of cursors across all backends and clear it on a switch**: Returning to a previously used backend requires downloading already merged files again. After rolling back to an older version, it may use a timestamp written by WebDAV to query Drive and miss files uploaded by other devices in the meantime (see §3).
- **Keep the old backend's credentials when switching**: Switching back would not require signing in again, but the app would need to handle two backends being signed in at once, both in the UI and in its state logic. It would also need to store another secret. A Nutstore app password can access the entire account.

## Consequences

### Benefits

- Switching between Google Drive and WebDAV, or changing WebDAV providers (for example, from Nutstore to Nextcloud), preserves local history without creating another database for the new backend.
- The same WebDAV account continues to use the same local database when its URL spelling or password changes.
- Each backend has independent sync progress, so a timestamp from one backend is never used to evaluate files on another. Sync can resume when returning to a previously used backend, and rolling back to an older version does not cause Drive files to be missed.

### Costs and risks

- Every backend switch re-uploads this device's activities and five whole-table files (see §2). The first time a backend is used, the app also downloads all of its data. This can generate substantial network traffic on devices with extensive history.
- Other devices still using the old backend will not see data on the new one. They must switch to the same backend to resume syncing with it.
- The same WebDAV account can be used by two local databases. For example, local database A has switched to WebDAV account W. Later, the user enters W in the local database for Drive account B. Because B's database has no record of W, the app keeps using B's database: it pulls A's data from W into B's database and uploads B's data to W. This can happen only if the user enters W's password in B's database.

## Data, compatibility, security, and privacy

- **Existing data and migration**: The database migration adds five columns to `auth_state`. For a local database whose filename contains a Google uid (meaning it has signed in to Google), initialize `backend = drive` and `drive_account = <that uid>`, whether or not the user is currently signed out. Leave both columns empty for anonymous databases. Existing cursor and push-fingerprint rows belong to Drive and keep their names. WebDAV has not shipped yet, so its new rows need no migration.
- **Mixed versions and rollback**: The migration only adds columns, so older versions that read existing fields by column name are unaffected. Older versions do not support WebDAV. After a user rolls back from WebDAV, the older version shows them as signed out. Once they sign in to Google again, syncing can resume from the progress recorded when they left Drive.

	Records created on this device while using WebDAV will not be uploaded to Drive by the older version, because their outbox rows were removed after being pushed to WebDAV. When the app is upgraded again, if `backend` is still `webdav` but the local database contains a Google token, the user signed in to Drive using the older version. In that case, handle the transition as a switch from WebDAV to Drive and run the transaction in §2: refill the outbox to upload records created while using WebDAV, and delete the WebDAV password.
- **Irreversible effects**: None. Switching backends does not delete sync progress or business data on this device.
- **Security and privacy**: Only the encrypted WebDAV password is stored. The account hash contains no plaintext, so filenames do not directly expose the server address or username. WebDAV must still use HTTPS, as required by ADR-0007.

## Follow-up

Implement in this order:

1. Move the three AES functions required for local encryption out of the Google module.
2. Add the `auth_state` migration.
3. Implement the WebDAV account hash and server host prefix, with tests using fixed inputs and outputs to lock down the normalization rules.
4. Store sync progress separately by backend: keep the existing row names for Drive and use the `webdav.<host>.` prefix for each WebDAV server. The backend determines the row names.
5. Implement the connect command and backend-switch transaction.
6. Update the Google sign-in flow to identify the account using `auth_state.drive_account`, as described in §1, instead of the uid in `active_user.json`.

Write a separate ADR later to unify the push mechanism: track a push cursor for each backend and remove the outbox. Then a backend switch will not need to refill the outbox; each backend can track which data it is missing. This depends on two changes:

- Use `updated_at` to determine which activity dates have changed. First, replace hard deletes (such as `purge_orphan_sessions`) with soft deletes; otherwise, the cursor cannot see deleted rows. As with pull cursors, the cursor must also look back over a time window to avoid missing records if the system clock moves backwards.
- Split AI summaries and chat history into per-day files, so only files for changed days need to be uploaded. This changes cloud filenames, so compatibility is needed for older versions that recognize only `ai_summaries.json` and `chat.json`.
