# ADR-0009 · WebDAV uploads and manifests (supersedes ADR-0008)

- **Date**: 2026-09-23
- **Status**: **Accepted**
- **Related**: Supersedes ADR-0008 · supersedes ADR-0007 §6's single-file upload method · ADR-0005 (whole-file rewrites) · ADR-0006 (one pull cursor per dataset) · PR #56 (cursor rewind)

## Context

After ADR-0008 was implemented, end-to-end tests and code review found five user-visible problems:

| Problem | Root cause |
|---|---|
| 1. When a dataset is turned on after having been off, it cannot pull the peer's old history. For example, if AI summaries were synced while the switch was off, turning the switch on still does not fetch the peer's existing summaries. | Each peer has only one pull progress value, shared by all four datasets. |
| 2. If one device pushes twice within the same second, other devices miss the files changed in the second round. There is no error; those files only show up again after they are changed once more. | Server time is only second-precision, so two manifests written in the same second cannot be told apart. |
| 3. If the manifest upload succeeds but the local database has not recorded it yet, or if the response is lost in flight, the next round reuses the same push count and other devices miss files. | The device keeps its own push count locally, and there is no atomic transaction between that state and the cloud. |
| 4. A fresh Nutstore account cannot sync at all: directory listing returns 404 and the first upload returns 409. | Nothing creates the sync root directory. |
| 5. A single file uploaded with plain `PUT` can be read half-written by other devices if the connection drops. | Single files do not go through a temporary name (ADR-0007 accepted this risk in §6). |

## Decision

Keep the ADR-0008 manifest file design, but make five changes that map directly to the problems above:

| Change | Problem solved |
|---|---|
| 1. Each upload replaces a file the way the server allows: a plain `PUT` on Nutstore, a temporary name and a `MOVE` elsewhere | Problem 5 |
| 2. The device no longer stores its own push count; it reads the manifest from the cloud each time | Problem 3 |
| 3. Two manifest uploads must be at least one second apart | Problem 2 |
| 4. Pull progress, called a bookmark, is tracked separately per dataset | Problem 1 |
| 5. The sync root directory is created on demand | Problem 4 |

The rest of this ADR spells out the rules in full so that ADR-0008 does not need to be read again.

### 1. Upload: replace a file the way the server allows

Other devices must see either the old content of a file or the new, never half of it. Servers differ in how that can be done, so the client picks the method from the host in the server address. The same method applies to day files, single files and manifest files.

| Server | Method | Why |
|---|---|---|
| Nutstore (`dav.jianguoyun.com`) | `PUT` straight onto the file name | Nutstore swaps a file in only after the whole body has arrived. Measured: a reader during an upload gets the old content, and an upload cut off halfway leaves the old file, or no file, behind. Its `MOVE` refuses to overwrite: 409 `DuplicateName`, with or without `Overwrite: T`. |
| Every other server | `PUT` to `.tmp-<file-name>` in the same directory, then `MOVE` onto the file name with `Overwrite: T` | `PUT` is not atomic everywhere: an interrupted `PUT` on Nextcloud 32 leaves the truncated body as the live file ([nextcloud/server#62321](https://github.com/nextcloud/server/issues/62321)). Nextcloud, nginx and Apache replace the target on `MOVE`. |

The server type is not a user setting: a wrong choice on Nextcloud would silently truncate files, while the address already tells Nutstore apart. A server that neither replaces a file on `MOVE` nor swaps a `PUT` in whole makes the `MOVE` fail with an error, so nothing is written half; such a server needs a method of its own.

If `PUT` returns 409, the parent directory does not exist. Starting from the sync root, create missing parent directories one level at a time with `MKCOL` (a 405 means the directory is already there), then retry the `PUT` once. If it still fails, return the error. Fresh accounts use the same rule to create the sync root.

### 2. Manifest format

Each device has one manifest file in the sync root:

```text
/hindsight/manifest.<device-id>.json
```

```json
{
  "version": 1,
  "files": {
    "categories.json": 42,
    "activities/2026/2026-09-20.ndjson": 41
  }
}
```

- `version`: the format version. An unknown version is an error; it is not parsed as the current format.
- `files`: every file this device has published; the key is a path relative to the device directory, and the value is the push count at which that file was last uploaded.
- `push count`: the device's own counter. It increases by one every time a manifest is uploaded.

The manifest carries no device clock. Each device uploads only its own manifest.

### 3. Push: record file changes and update the manifest

A push round first uploads the changed data files, and only then uploads the manifest.

**Record data-file changes.** As soon as a data file upload or delete succeeds, record it in the local `sync_cursor` row `webdav.pending`. That row stores data files that have already been uploaded or deleted but are not yet written into the manifest, so it survives app restarts.

The upload cannot miss the record because of the ordering: `upsert_by_name` writes `webdav.pending` before it returns success, and the engine only clears its own work after that success (by deleting the outbox row or advancing the dataset watermark). If anything fails in between, the pending entry is still there; the next round uploads again and records it again, so the worst case is one extra upload.

Deletes do not have that guarantee yet: the command that removes data from the cloud has no pending entry. If the server delete succeeds and the local record does not, the manifest keeps listing the file. This needs to be solved together with WebDAV's "remove this device from the cloud" feature.

**Upload the manifest.** This is the last step of every push round. If `webdav.pending` is non-empty, run the following five steps. Do this even when some data-file upload failed in the same round: the failed file is not in `webdav.pending`, and when it later succeeds it will be recorded under a larger push count.

1. Read this device's manifest from the cloud. A 404 means there is no prior manifest and it should be treated as empty. With the temporary-name method, a 404 first falls back to `.tmp-manifest.<device-id>.json`: a server may delete the target of a `MOVE` before moving, and a failure in between leaves only that copy, which is the newest version.
2. Compute this round's push count: take the maximum push count already present in the manifest and add one. The files from the previous round were recorded under the previous count, so the maximum is the previous round's count. For example, if the manifest contains `{"categories.json": 5, "icons.json": 3}`, this round's push count is 6.
3. Record the uploaded data files from `webdav.pending` under that count, and remove deleted files from `files`.
4. Upload the manifest.
5. Clear `webdav.pending` only after the manifest upload succeeds; if it fails, keep the pending entry and retry the same content next round.

**Do not store a local manifest copy; read it from the cloud every time.** HTTP and SQLite are not in one transaction, so a local copy can lag behind the cloud, for example if the upload succeeds but the local write does not, or if the response is lost. If the count from the lagging copy is reused, other devices skip new files. Using only the cloud copy avoids that mismatch.

**Keep at least one second between manifest uploads.** Count from the end of the previous manifest upload, and count failures too, because a failure may only mean the response was lost. Pulling relies on server time to distinguish manifest versions, and server time is only second-precision: two versions written in the same second would be treated as already handled.

### 4. Pull: track bookmarks per dataset

This device keeps one **bookmark** per peer and per dataset. A bookmark has two parts: the peer's push count that has already been processed, and the server time of the manifest version at the time it was recorded. The datasets are the engine's four pull streams: core, AI summaries, chat and screen memory (ADR-0006).

A pull round has five steps:

1. Send `PROPFIND` with `Depth: 1` to the sync root to get every manifest and its server time. A 404 means this is a new account and there is nothing to pull in this round.
2. The engine passes in the datasets that are currently enabled, together with their local cursors.
3. For each peer manifest, look only at the enabled datasets whose bookmark time differs from that manifest's server time.
4. If none differ, skip that peer and do not `GET` the manifest. Otherwise `GET` the manifest and process those datasets one by one:
   - If the local cursor has already reached "manifest time minus one second", the engine has finished merging that version. Update the bookmark to the largest push count in that version together with that version's server time, and do not report files.
   - Otherwise, report the files for that dataset whose push count is greater than the count stored in the bookmark. Give all of them the manifest's server time.
5. Disabled datasets keep their bookmarks unchanged; when they are enabled later, they resume from their own bookmarks.

The reason for "minus one second" is that all files in one version are reported to the engine with the same time, and after merging the engine rewinds its cursor by one second (PR #56).

## Alternatives

- **One progress value per peer (the ADR-0008 design)**: simple, but when one dataset advances the progress, disabled datasets are treated as already handled.
- **Split the manifest into several files by dataset**: does not solve the problem. The issue is on the read side: with only one cursor, it is impossible to tell which dataset is disabled; it also means more uploads and more `GET`s.
- **Only change pull so that whenever a dataset lags behind, handled files are reported again with "bookmark time minus one second"**: this avoids changing the API, but it needs a "minus two seconds" threshold to avoid re-reporting on every round, couples the design tightly to the engine's rewind, and also forces the core cursor backward.
- **Keep a local copy of the manifest**: saves one `GET` in a round with changes, but the local copy can lag behind the cloud, as described above.
- **Plain `PUT` on every server**: one request per file, but on Nextcloud an interrupted upload leaves a truncated file for other devices to read.
- **Temporary name and `MOVE` on every server**: Nutstore refuses `MOVE` onto an existing file. Deleting the target first costs four requests per rewrite and leaves a moment with no file.
- **A hash of every file in the manifest, checked by the reader**: works on any server, but changes the manifest format and is not needed once each server gets its own method.
- **Let the user choose the server type**: a wrong choice on Nextcloud fails silently, and the address already tells Nutstore apart.

## Consequences

- **Benefits**: datasets that were disabled can catch up when enabled; cleared data does not come back; same-second writes, lost responses and crashes do not make peers miss files; fresh accounts work immediately; peers never read half-written files.
- **Costs**:
  - A push round with changes uploads the manifest too: a `GET` and a `PUT` on Nutstore, plus a `MOVE` elsewhere. Outside Nutstore every uploaded file also costs one extra `MOVE`.
  - A push round that follows another one may wait up to one extra second.
  - If bookmarks are lost, peer files are processed again. Merging is idempotent, so nothing is duplicated.
  - If the device crashes after uploading the data files but before uploading the manifest, `webdav.pending` is still there, so the next push fills the gap and the peers only see the files after that.
- **Still unsolved**: if a device's files are deleted from the cloud, for example after removing that device and later signing in again with the same account, or after the user deletes the directory in the web UI, the manifest starts counting from 1 again and peers skip new files whose push count is not greater than the old bookmark. This needs to be designed together with WebDAV's "remove this device from the cloud" feature.

## Data, compatibility, and security

- **Existing data**: WebDAV has not shipped, so there is nothing to migrate. Local state reuses the `sync_cursor` table and adds two row kinds (see `docs/design/database.md`), so no migration is needed. Google Drive behavior stays the same: the backend's file-listing interface changes to accept per-dataset cursors, but Drive still lists from the earliest one.
- **Mixed versions and rollback**: WebDAV ships with this design the first time it is released; the manifest format is the same as in ADR-0008, and `version` remains 1.
- **Irreversible effects**: see "Still unsolved" above; local data is unaffected.
- **Security and privacy**: the manifest contains only relative paths and push counts, not file contents, so the information it reveals is the same as a directory listing.

## Follow-up

1. Run the two `probe_` tests against Nextcloud as well; they pass on Nutstore (2026-09-23).
2. Design WebDAV's "remove this device from the cloud" and "forget the remote device": list all files for one device from the manifest and decide what happens to the manifest and push counts after deletion.
3. Clean up leftover `.tmp-` files (ADR-0007).
