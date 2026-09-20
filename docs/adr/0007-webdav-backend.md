# ADR-0007 · Support a WebDAV backend (Nutstore and others)

- **Date**: 2026-09-20
- **Status**: **Accepted**
- **Related**: ADR-0002 (deriving the sync key from the backend's credential) · ADR-0005 (whole-file rewrites) · ADR-0006 (one pull cursor per dataset) · `sync::drive::DriveBackend`

## Context

Sync supports Google Drive only. A WebDAV backend is added so that Nutstore,
Nextcloud, ownCloud and self-hosted NAS boxes can be used as well.

The sync engine reaches the cloud through `DriveBackend`, which lists,
downloads, writes and deletes files. WebDAV covers those with `PROPFIND`,
`GET`, `PUT`, `DELETE` and `MKCOL`, and authenticates with HTTP basic
authentication. To keep a file that is still uploading from being read, the
WebDAV backend also needs `MOVE` within one server; other backends provide an
equivalent way of committing a file.

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

A dataset whose switch is off takes no part in listing, downloading or merging.
Both the listings and the file operations count against the request budget;
this design sets the WebDAV sync interval to 5 minutes.

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
declares its time precision; when the engine writes a cursor it moves the
cursor back by one unit of that precision, so the next round re-examines the
files of that last second.

Downloading a file again produces no duplicates, since every merge is required
to be idempotent. ETags are deliberately not used to detect changes: that would
add a table of per-path ETags and a second branch in the "have I handled this
file" test. If repeated downloads or the request budget turn out to be a
problem, ETags can be weighed on their own.

### 5. No readme file in the sync directory

The directory is visible to the user. A readme would not stop anyone from
deleting or changing the sync files and would be one more thing to maintain.

### 6. Day files are uploaded under a temporary name, then moved into place

A Drive upload switches the file over on the server side, so other devices
never read a half-written file. Most WebDAV servers write a `PUT` as it
arrives: a dropped connection leaves a truncated file under the real name, the
other devices fail to parse it on their next round, and it heals only when the
source device rewrites the whole file again (ADR-0005).

Day files — activities and screen memory — are therefore `PUT` to a temporary
name in the same directory (`.tmp-2026-09-20.ndjson`) and, once the server has
answered 2xx, `MOVE`d onto the real name, which no other device reads. A `MOVE`
within one server and one directory generally avoids exposing an unfinished
file, but whether it is atomic has to be verified against the target provider;
across servers a `MOVE` may degrade into a copy and a delete and cannot be
relied on.

The single files under the device directory (categories, app groups, icons,
device meta, tombstone, AI summaries, chat) are `PUT` directly, accepting that
incomplete content is briefly visible while they upload. If that turns out to
make other devices read broken files often, they move to the same two-step
commit.

The request carries `Content-Length`, and a 2xx means the request was accepted;
it is not a promise that every provider verified and persisted the bytes. The
temporary name only closes the window in which an unfinished file is visible to
other devices; it does not verify what was uploaded.

A crash can leave a `.tmp-` file in the cloud. Listings ignore names starting
with `.tmp-`; cleaning up orphaned temporary files older than a retention
period comes later.

Directories are created on demand rather than up front: `PUT` straight away,
and only on a 409 (the parent does not exist) create the missing levels with
`MKCOL` and retry the `PUT` once. `MKCOL` creates one level at a time and
answers 405 when the directory is already there, so creating them up front
would add useless requests to every round.

## Error handling

The existing rules stand: a single file that fails to download or merge is
logged and does not advance the cursor, so the next round retries it; a failed
listing fails the whole round and is written to `last_error`; a failed upload
is retried through the outbox with backoff. No download failure advances the
cursor, 404 included — a file that really was deleted is absent from the next
listing, and the cursor passes its place on its own. The WebDAV status codes
map as follows:

| Status | Meaning | Handling |
|---|---|---|
| 401 | Wrong user name or app password, or the password was revoked | Treated as an invalid credential: ask the user to enter it again. No retry — there is nothing to refresh |
| 429 / 503 | The provider is rate-limiting, or is briefly unavailable | Back off by `Retry-After` when it is given, otherwise skip this round; never retry straight away. A failed listing fails the round; a failed file operation is retried according to its kind |
| 404 | The file or directory is not there | Download: as with any download failure, skip it this round and leave the cursor; delete: treated as success (idempotent); listing: an empty directory, for example `memory/2027/` before the year turns |
| 405 | `MKCOL` on a URL that already exists (RFC 4918: `MKCOL` only applies to an unmapped URL) | Treated as "the directory is already there"; a 405 from any other method is an error |
| 409 | The parent directory of a `PUT` does not exist | Create the missing levels with `MKCOL` and retry that `PUT` once; if it still fails, return the error rather than looping |
| 507 | The cloud account is out of space | Reported to the user and marked as a permanent failure; no pointless automatic retries |
| Timeout / no network | A network problem | Retryable: the next round tries again |

## Alternatives

- **A manifest file per device**: fewer listing requests, but the manifest can
  disagree with the files that are actually there. Not taken.
- **Every file flat in one directory**: simpler, but once the files pile up it
  needs directory paging. Not taken.
- **A Nutstore-only integration**: a smaller job that supports one provider
  instead of the protocol. Not taken.

## Consequences

- Nutstore, Nextcloud, ownCloud and self-hosted WebDAV services become usable.
- Sync latency on WebDAV is about 5 minutes, against 30 seconds on Drive.
- Every day-file upload costs one extra `MOVE` request, which counts against
  the budget.
- The backend interface grows a commit or `MOVE` step, which every backend has
  to provide in some equivalent form.
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
6. Check whether Nutstore's `PUT` returns an `ETag` that is documented to be the
   content's MD5. Only a provider guarantee makes it usable for verifying an
   upload; if it is a version identifier or anything else, do not infer content
   equality from it.
