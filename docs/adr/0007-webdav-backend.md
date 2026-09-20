# ADR-0007 · Support a WebDAV backend (Nutstore and others)

- **Date**: 2026-09-20
- **Status**: **Accepted**
- **Related**: ADR-0002 (deriving the sync key from the backend's credential) · ADR-0005 (whole-file rewrites) · ADR-0006 (one pull cursor per dataset) · `sync::drive::DriveBackend`

## Context

Sync only supports Google Drive today. A WebDAV backend is added so that
Nutstore, Nextcloud, ownCloud and self-hosted NAS boxes can be used as well.

The sync engine reaches the cloud through `DriveBackend`, which lists,
downloads, writes and deletes files. WebDAV covers those with `PROPFIND`,
`GET`, `PUT`, `DELETE` and `MKCOL`, and authenticates with HTTP basic
authentication.

Four differences from Google Drive bear on the current design:

1. Google Drive can filter by `modifiedTime`; a WebDAV `PROPFIND` returns the
   contents of a directory, so the incremental filtering has to happen on the
   client.
2. WebDAV's `getlastmodified` is usually second-precision, while the pull
   cursor advances by file modification time. When several files are written
   within the same second, a strict greater-than comparison can skip one.
3. WebDAV has no equivalent of Drive's `appDataFolder`. The sync files sit in
   an ordinary directory, where the user can see, change or delete them.
4. Nutstore limits how many requests may be made and how many entries a single
   directory listing returns. The directory layout and the sync interval have
   to keep the listing requests in check; uploads, downloads and retries are
   not part of the listing estimate below and need their own handling in the
   implementation.

## Decision

### 1. A layout split by device and year

The cloud directory is laid out like this:

```text
/hindsight/
  <device-id>/
    meta.json
    categories.json
    app_groups.json
    app_group_members.json
    icons.json
    tombstone.json
    ai_summaries.json
    chat.json
    activities/2026/2026-09-20.ndjson
    memory/2026/2026-09-20.ndjson
```

Activities and screen memory are split by year, so a year directory holds at
most 366 day files. The other datasets are a single file under the device
directory.

The WebDAV backend translates between those paths and the flat file names the
engine uses:

```text
device.<id>.activities.2026-09-20.ndjson
```

A dataset whose switch is off takes no part in listing, downloading or
merging. Both the listing requests and the file operations count against the
request budget; this design sets the WebDAV sync interval to 5 minutes.

### 2. One backend per local sync database

Only devices on the same cloud backend sync with each other. Switching backends
is treated as switching accounts and goes through the existing database switch;
the old database stays on the machine.

The local account identity for WebDAV is derived from the server address and
the user name, and selects the local database. The exact rule is fixed by the
implementation.

### 3. WebDAV syncs every 5 minutes

Google Drive keeps its 30-second interval; WebDAV uses 5 minutes to hold down
the listing requests. The budget cannot be estimated from listings alone —
uploads, downloads and the retry policy have to be checked during
implementation.

### 4. Keep the time cursor, with an overlapping window for WebDAV

Because WebDAV times are second-precision, the last time handled in a round
cannot be used as a strict starting point for the next one. The backend
declares its time precision; when the engine writes a cursor it moves it back
by one unit of that precision, so the next round re-examines the files of that
last second.

Downloading a file again produces no duplicates, since every merge is required
to be idempotent. ETags are deliberately not used to detect changes: that would
add a table of per-path ETags and a second branch in the "have I handled this
file" test. If repeated downloads or the request budget turn out to be a
problem, ETags can be weighed on their own.

### 5. No readme file in the sync directory

The directory is visible to the user. A readme would not stop anyone from
deleting or changing the sync files and would be one more thing to maintain.

## Alternatives

- **A manifest file per device**: fewer listing requests, but the manifest can
  disagree with the files that are actually there. Not taken.
- **Every file flat in one directory**: simpler, but once the files pile up it
  needs directory paging, which each provider handles differently. Not taken.
- **A Nutstore-only integration**: a smaller job that supports one provider
  instead of the protocol. Not taken.

## Consequences

- Nutstore, Nextcloud, ownCloud and self-hosted WebDAV services become usable.
- Sync latency on WebDAV is about 5 minutes, against 30 seconds on Drive.
- The cloud files are visible to the user. Nothing prevents them from deleting
  or editing them, and the cloud copy may not be recoverable afterwards.
- Authentication, directory operations, rate limiting, retries and provider
  quirks all become ours to maintain.

## Data, compatibility, and security

- Existing Google Drive users keep their database and their sync behaviour.
  WebDAV uses a new local sync database; no Drive data is migrated.
- An older version does not know about the WebDAV configuration, so a user who
  goes back can only use Google Drive. The WebDAV database stays on disk and
  works again on a version that supports it.
- WebDAV must run over HTTPS; a plain HTTP connection is refused.
- The WebDAV app password is kept in the local settings and can be revoked on
  its own from the provider's site. It must never reach a log or an error
  message.
- Per ADR-0002 the app password is the backend credential the sync key is
  derived from: the user types it into each device, and it never passes through
  the sync provider.

## Follow-up

1. Adjust the backend abstraction so WebDAV can implement the existing file
   operations.
2. Implement the WebDAV client, with a fake server for the tests.
3. Implement credential storage, the HTTPS check, rate limiting and backoff.
4. Add a connection test and the backend choice in the UI.
5. Work out the error handling once a real provider's rate-limit responses have
   been seen.
