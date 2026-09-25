# ADR-0011 · Switching backends and WebDAV accounts

- **Date**: 2026-09-23
- **Status**: **Accepted**
- **Related**: Supersedes ADR-0007 §2 ("switching backends is switching accounts") and what ADR-0007 says about the new database, the password and the sync key · ADR-0002 (local key) · ADR-0005 (whole-file rewrites) · ADR-0006 (one pull cursor per dataset) · ADR-0009 (WebDAV bookmarks)

## Context

The rule so far has been one local database per account. Switching Google accounts switches to another database, so the data of two accounts never mix.

With WebDAV, a backend and an account are no longer the same thing: a user may move from Google Drive to WebDAV, and back again. The old design treated switching backends as switching accounts and started an empty database each time. The history and screen memory stayed behind in the old database, out of sight after the switch, and the old database kept taking disk space.

ADR-0007 also left open how a WebDAV account is identified and where its password is stored:

- The password cannot go into the ordinary settings, because `get_settings` returns the whole settings object to the frontend.
- The local key of ADR-0002 only encrypts credentials stored on this machine; it is not a sync key. Cloud files are not encrypted with it.

## Decision

### 1. Tell switching backends apart from switching accounts

The two are handled differently:

- **Switching backends**: keep the current local database, clear the old backend's sync progress, and let the new backend sync this device's data and the cloud's data again.
- **Switching accounts on the same backend**: the current database cannot be kept. Create or open another database, so the two accounts' data do not mix.

Examples:

| Change | Local database |
|---|---|
| Drive account A → WebDAV account W | Keep the current one; clear the progress and sync again |
| WebDAV account W → Drive account A | Keep the current one; clear the progress and sync again |
| Drive account A → Drive account B | Create or open B's database |
| WebDAV account W → WebDAV account X | Create or open X's database |

Each local database remembers at most one Drive account and one WebDAV account. A user moving back and forth between the two backends therefore stays in the same database; only a different account on the same backend switches databases.

For example, a user moves from Drive account A to WebDAV, then signs in to Drive account B. B is not A, the Drive account this database recorded, so B gets another database; A's database is not reused.

### 2. A backend switch happens all at once

When switching from one backend to another, do these four things in one transaction on the current database:

1. Save the new backend and its credentials, and record the account for that backend.
2. Clear the four pull cursors and the three push fingerprints in `sync_cursor`. Keep the WebDAV bookmarks and pending changes: the database is the same, so the data merged from this WebDAV account is still there, and a database has only one WebDAV account. Coming back to it downloads only the files added while the database was away.
3. Refill the outbox: one row for each day this device has activity, and one row for each of the five single files. Push rewrites whole files (ADR-0005), so one row per file is enough.
4. Delete the old backend's credentials: the Google token or the WebDAV password.

Once the transaction commits, the normal push and pull loop does the rest:

- Push uploads this device's data to the new backend.
- Pull checks and merges the new backend's cloud data from the start.

Merging is an upsert, so processing the same data twice creates no duplicates.

### 3. Why a backend switch must sync again

Sync progress cannot be shared between backends, above all because they record times and find files differently.

For example, the old Drive cursor may read `11:05:01`. After switching to WebDAV, comparing it with the server time of a peer's manifest can make the version the peer published at `10:55` look already processed. The bookmark is updated, no file is downloaded, and the older days are never found later.

With the cursors cleared, the new backend rebuilds its progress from its own times, and no data is skipped silently. Syncing from the start deletes nothing on this device; it only uploads and checks the existing data again.

This step must be in the same transaction as saving the new backend. If the app crashes after the new backend is saved but before the cursors are cleared, the next start judges the new backend's data by the old cursors. All of this state is in the main database, so one transaction protects it.

### 4. What account information is stored

`auth_state` gains six columns:

- `backend`: the backend this database uses now, `drive` or `webdav`; empty if it has never synced.
- `drive_account`: this database's Drive account, the Google uid.
- `webdav_account`: this database's WebDAV account, the id defined in §5.
- `webdav_url`
- `webdav_user`
- `webdav_password_enc`

The two account columns do not change once written: switching backends does not overwrite them, and after switching accounts the new database has its own. Signing out deletes only the credentials (the Google token or the WebDAV password) and keeps `backend` and both account columns, so the next connection can still tell whether it is the same account.

The WebDAV password is encrypted with the local key of ADR-0002, the same protection as the Google refresh token.

### 5. The WebDAV account id

The WebDAV account id has this form:

```text
webdav-<first 16 hex characters of SHA-256>
```

The hash input is:

```text
normalized URL + newline + normalized user name
```

Normalization:

- URL: lowercase the scheme and host, drop the default port, drop the trailing `/`.
- User name: trim the surrounding whitespace and lowercase it.

For example, these two connections are the same WebDAV account:

```text
HTTPS://DAV.jianguoyun.com:443/dav/
https://dav.jianguoyun.com/dav
```

The function that computes the id must be tested with fixed inputs and outputs. If the rule ever changes, existing users get a new id after a restart, are taken for a new account, and land in an empty database.

## Alternatives

- **Create a new database on every backend switch**: the two backends' data never mix, but after the switch the user cannot see the history and screen memory in the old database, and it takes extra disk space.
- **Derive the account id from the user name and password**: changing the password gives a new id and an empty database. The password's hash would also stay on disk as part of a file name, where it can be cracked offline.
- **Use only the user name**: after moving to another server, the database keeps the old account's cursors and bookmarks and may skip files on the new server.
- **Remember only the last account per database**: a backend switch overwrites it. After moving from Drive account A to WebDAV and then signing in to Drive account B, the app cannot tell that B is a different account and reuses A's database.
- **Record outside the databases (`active_user.json`) which database each account belongs to**: this closes "one WebDAV account used by two databases" (see Consequences), but it keeps state outside the database, and that state cannot share a transaction with the backend switch. The case only happens when a database is given a password that another database already uses; it is not worth it.
- **Store a random account id in the cloud**: avoids the problems of URL spelling and password changes, but adds a protocol file. Normalizing the URL and the user name is enough for now.

## Consequences

### Benefits

- Switching between Google Drive and WebDAV loses no local history and does not create another set of databases.
- The same WebDAV account keeps the same local database when its URL is written differently or its password changes.
- With the old cursors cleared, the new backend builds its progress from the start and skips no data because of the old backend's state.

### Costs and risks

- The first switch to a backend uploads this device's history again and pulls the cloud data again. On a device with a lot of history, that round uses noticeable network traffic.
- Other devices still on the old backend do not see the data on the new backend; they sync again only after they switch to the same backend.
- One WebDAV account can be used by two local databases. For example, A's database has switched to WebDAV account W; later W is also entered in the database of Drive account B. B's database has no WebDAV record, so it is kept: A's data on W is pulled into it, and B's data is uploaded to W. This only happens if W's password is entered in B's database.

## Data, compatibility, security, and privacy

- **Existing data and migration**: The migration only adds six columns to `auth_state`. A database whose file name carries a Google uid (only a database that has signed in to Google has one) gets `backend = drive` and `drive_account = <that uid>`, whether or not it is signed out now. An anonymous database leaves both empty.
- **Mixed versions and rollback**: The migration only adds columns, and older versions read the existing columns by name, so they are not affected. Older versions do not support WebDAV. A user on WebDAV who rolls back to an older version is shown as signed out, and can keep using Drive after signing in to Google again.
- **Irreversible effects**: The cursors and push fingerprints cleared by a backend switch cannot be recovered, but the next round rebuilds them. No data on this device is deleted.
- **Security and privacy**: Only the encrypted WebDAV password is stored. The account id is a hash, so file names do not show the server address or the user name. WebDAV must still use HTTPS, as ADR-0007 requires.

## Follow-up

Implementation order:

1. Move the three AES functions used for local encryption out of the Google module.
2. Add the `auth_state` migration.
3. Implement the WebDAV account id, with tests for the normalization rules.
4. Implement the connect command and the backend switch transaction.
5. Change the Google sign-in flow to decide by `auth_state.drive_account`, as in §1, instead of the uid in `active_user.json`.
