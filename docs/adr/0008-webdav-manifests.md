# ADR-0008 · WebDAV pull reads per-device manifests instead of listing directories

- **Date**: 2026-09-21
- **Status**: Superseded by ADR-0009
- **Related**: ADR-0005 (whole-file rewrites) · ADR-0006 (one cursor per dataset) · ADR-0007 (WebDAV backend; this ADR replaces its directory-walking pull, the layout stands) · PR #56 (cursor rewind)

## Context

ADR-0007 has WebDAV pull walk every device directory and every year
directory, then filter the files by modification time on the client.

The number of directories grows with the number of devices and years. With
two devices and two years of data a round needs about 15 `PROPFIND`
requests, and every round re-downloads the listing of directories that have
not changed: an estimated 0.6–1.2 MB per round. These numbers have not been
measured on Nutstore, but the growth with data size is certain.

Listing only recent directories does not help. Activities and screen memory
are stored by date, but a file for an old date is rewritten whole whenever a
row of that day changes or is deleted (ADR-0005). A rewrite in a directory
pull did not list is never seen. Whether a directory's own modification time
changes when a file inside it is rewritten depends on the server and cannot
be relied on.

Nutstore's request budget is fixed. An idle pull must stop walking the data
directories.

## Decision

Every device writes its own manifest to the sync root after each push round
that uploaded all its files:

```text
/hindsight/manifest.<device-id>.json
```

WebDAV pull lists the sync root only, never the device or year directories.
The manifests are the only way changes are discovered.

### Manifest format

```json
{
  "version": 1,
  "files": {
    "categories.json": 42,
    "activities/2026/2026-09-20.ndjson": 41
  }
}
```

- `version`: the manifest format version.
- `files`: every file the device publishes, with the push count at which it
  last changed. The push count is the device's own counter; it goes up by one
  for every push round that uploaded at least one file. Paths are relative to
  the device directory. Single files and day files are both listed.

A manifest carries no device clock. Only the owning device writes its
manifest, after the round's files have all been uploaded, with a plain `PUT`.

### Pull

1. One `PROPFIND` (`Depth: 1`) on the sync root returns every manifest with
   its server modification time.
2. A manifest is fetched with `GET` only when its server modification time
   differs from the one this device recorded for the last version it fully
   processed. Same file, same server clock, so this is an equality check; the
   cursor is not involved. A round in which nothing changed makes this one
   request and no other.
3. From `files`, the files whose push count is above the count this device
   has processed for that peer are the ones to pull. This step alone decides
   what is pulled; `modified_after` plays no part in it.
4. Those files go to the existing sync engine with a name, a file id and a
   modification time. The modification time is the manifest's server
   modification time for every file.
5. The engine keeps its cursor rules, including the rewind of ADR-0006 and
   PR #56. No device clock takes part in ordering or in deciding what is new.

For each peer, this device keeps two values:

- the push count it has fully processed, which is the largest count in that
  version of `files`;
- the server modification time of that manifest version.

`modified_after` has one use here: the engine tells the client how far it
got. At the end of a round the engine stores its cursor as the last handled
time minus one second (PR #56). So when the next `modified_after` is at or
after a manifest version's server time minus one second, every file of that
version was handled; only then does the processed push count advance, and
that version's server time is recorded. When a file failed to download or
merge, the engine's cursor stops earlier, the condition is not met, and the
push count stays. The next round fetches the manifest again in step 2 and
reports those files again.

### Failures and compatibility

- A manifest that is missing or cannot be read: that peer is skipped for the
  round and the fact is logged. There is no fall-back to directory listing.
- Once the peer's next push writes its manifest, later pulls recover on
  their own.
- The backend interface gains an "end of push round" hook that writes the
  manifest. Google Drive's implementation does nothing.

## Alternatives

- **Walk every directory**: no manifest state, but the request count and the
  response size grow with the number of devices and years.
- **Walk the last two years only**: simple, but a rewrite of an older day
  file is missed without any error.
- **Record the device's own `updated_at` in the manifest**: device clocks
  differ; a file from a slow clock sorts before the cursor and is skipped. A
  push count that only goes up needs no clock.
- **Keep the manifest inside the device directory**: one `PROPFIND` per
  device to learn whether it changed. At the sync root, one request covers
  every device.
- **Use directory modification times to skip unchanged year directories**:
  server behaviour, unverified; not something sync can rest on.

## Consequences

- An idle pull goes from walking several directories to one `PROPFIND` of
  the sync root. The request count no longer depends on the number of years.
- One more file in the cloud per device. Two more pieces of local state: the
  processed push count per peer, and this device's own push count per file.
- A manifest grows with the file count: about 50 bytes per entry, so two
  years of data is about 35 KB. It is downloaded only when its server
  modification time changes.
- A push round that uploaded files makes one more `PUT`, for the manifest.
- If a device crashes after uploading its files and before writing the
  manifest, the other devices see those files only after its next push.
- If the local push-count state is lost, the files in the peers' manifests
  are processed again. Merging is idempotent, so nothing is duplicated.

## Data, compatibility, security, and privacy

- **Existing data and migration**: WebDAV has not shipped, so there is no
  WebDAV data to migrate. Google Drive writes no manifest and behaves as
  before. The local migration only adds the state above.
- **Mixed versions and rollback**: older versions do not know WebDAV. This
  design ships together with the WebDAV backend; there is no version of the
  backend that does not write manifests. The manifest carries `version`: a
  format the reader does not know is an error, never parsed as the current
  one.
- **Irreversible effects**: if the user deletes a manifest in the cloud, that
  device's changes are invisible until its next push rewrites the manifest.
  Local data is untouched.
- **Security and privacy**: a manifest holds relative paths and push counts,
  never file content. It exposes what a directory listing exposes.

## Follow-up

1. Implement manifest reading and writing in the WebDAV client, and hook the
   write into the push round of ADR-0007.
2. Add tests for the root `PROPFIND`, manifest parsing and failure recovery.
3. Measure on Nutstore: the size of the root listing, requests per round,
   and how often manifests are downloaded.
4. Check whether the cleanup of orphaned `.tmp-` files from ADR-0007 can use
   the manifest.
