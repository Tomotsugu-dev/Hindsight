# ADR-0010 · Drive syncs every 5 minutes

- **Date**: 2026-09-23
- **Status**: **Accepted**
- **Related**: Supersedes the Drive interval in ADR-0007 §3 · ADR-0005 (whole-file rewrites) · PR #64 · commit `59a6024`

## Context

Drive used to push every 30 seconds and pull every 60 seconds.

The problem is that a push does not upload only what is new. Under ADR-0005, a day file is rewritten and uploaded in full every time it changes. On a busy day, the same activity day file is uploaded again on almost every round. ADR-0005 measured one day file at about 190 KB; rewritten that often, it adds up to tens to a hundred megabytes of upload on a busy day, and other devices download the file again every time they pull.

The 30-second push interval also means many repeated requests: up to 120 pushes an hour, and each one usually adds only a few sessions that just ended.

ADR-0007 §3 set the 5-minute interval for WebDAV only and left Drive on its shorter one. Drive now uses 5 minutes as well, to cut the repeated uploads and downloads of whole day files.

## Decision

Drive pushes and pulls every **5 minutes**, the same as WebDAV.

This changes only when background sync runs. The format and content of sync files and the merge rules stay the same.

## Consequences

### Benefits

- On a busy day, the day file is uploaded at most a tenth as often as before.
- Each peer downloads it again at most a fifth as often.
- Drive and WebDAV use the same background interval, so the backends behave more alike.

### Costs and risks

- A record takes up to about 10 minutes to reach another device: up to 5 minutes for this device to push, then up to 5 minutes for the other device to pull. The average is about 5 minutes.
- The worst case used to be about 1.5 minutes, so sync feels noticeably slower.
- "Sync now" on the Devices page and the sync at app start are not affected; each still runs a round right away.
- Quitting the app does not push. Records from the last 5 minutes before quitting may wait until the next start to be uploaded; before, the wait was at most about 30 seconds.

## Data, compatibility, security, and privacy

- **Existing data and migration**: None needed. Cloud file names and formats are unchanged.
- **Mixed versions and rollback**: The interval only decides when this device pushes and pulls; it is not part of the protocol. Old and new versions keep syncing with each other. Rolling back returns Drive to its 30-second push and 60-second pull.
- **Irreversible effects**: None.
- **Security and privacy**: No change.

## Follow-up

- If users report that records made before quitting are not uploaded in time, consider adding a setting in the app to change the push and pull intervals.
