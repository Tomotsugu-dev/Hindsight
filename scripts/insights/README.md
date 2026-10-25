# Internal GitHub Insights collection

The `Internal GitHub Insights` workflow runs daily at **12:17 Asia/Tokyo** and can
also be started with `workflow_dispatch`. It reads repositories listed in
`config.json` and writes only to a private Cloudflare KV namespace. Traffic data
is never committed to this public repository or uploaded as an Actions artifact.

## Credentials

Store these as repository Actions secrets:

- `GH_INSIGHTS_TOKEN`: a fine-grained PAT restricted to the listed repositories,
  with **Administration: read** (Traffic) and **Contents: read** (release assets).
  The token's user must have write access to the target repositories.
- `CLOUDFLARE_API_TOKEN`: the existing deployment token, with KV write access.
- `CLOUDFLARE_ACCOUNT_ID`: the existing Cloudflare account ID.

Set `kvNamespaceId` in `config.json` to the namespace bound to the dashboard
Worker. Do not put credentials in this file. Actions' built-in `GITHUB_TOKEN`
does not expose the Administration permission needed by the Traffic API.

## Data and failure behavior

- `history:v1:owner/repo` stores merged daily view/clone records and one repository,
  source and path snapshot per UTC collection date. Each snapshot retains a
  legacy all-asset `releaseDownloads` total and installer-only `installerDownloads`
  totals for Windows, macOS, Linux and other platforms. Full release/asset details
  remain only in the latest two dates for current version summaries. This prevents
  the same large release catalog from filling the single-value KV limit over time.
- Existing release snapshots are migrated to totals during the next collection.
  A successful empty release list means zero downloads; absent or invalid legacy
  release data remains unknown (`null`) rather than becoming zero.
- Installer counts include only `.exe`, `.msi`, `.msix`, `.msixbundle`, `.appx`,
  `.appxbundle` (Windows), `.dmg`, `.pkg` (macOS), `.deb`, `.rpm`, `.AppImage`
  (Linux), and `.apk` (other), case-insensitively. Signatures, JSON manifests,
  ZIP/tar archives and unrecognized files are excluded. Old all-asset totals
  cannot be used to reconstruct installer-only counts after details were removed.
- Daily installer increases are calculated only between adjacent UTC collection
  dates with known, nondecreasing cumulative counts. The first date, missing-day
  gaps, unknown data and counter resets produce an unknown increase (`null`),
  never a fabricated zero. These are download counts, not unique users or installs.
- `snapshot:v1:owner/repo:YYYY-MM-DD` stores the corresponding daily collection as
  a separate recovery copy, including the full release catalog for every date.
  History compaction does not change these copies. A manual rerun replaces only
  that same date's copy.
- `status:v1` stores the attempt time, previous complete success time,
  per-repository success times, and sanitized failures.
- Every run rereads GitHub's last 14 days. It replaces overlapping calendar days
  instead of adding rolling totals, preserves older history, and never turns a
  missing response into zero. A failed repository retains its last good history.
- Source/path rankings remain **rolling 14-day Top 10 snapshots**. They are not
  daily attribution. Daily unique visitors cannot be added to obtain deduplicated
  monthly visitors. Clone counts exclude fetches. `openIssues` includes open PRs.
- The raw daily recovery copy is written before the merged history. Status is
  updated last. Jobs are serialized to avoid competing archive updates. KV can
  take approximately 60 seconds to become visible across locations.

## Local checks

```sh
node --test scripts/insights/lib.test.mjs scripts/insights/downloads.test.mjs
node scripts/insights/collect.mjs --dry-run
node scripts/insights/collect.mjs --output scripts/insights/output
```

The latter two need `GH_INSIGHTS_TOKEN` in the environment and do not contact
Cloudflare. `--dry-run` keeps records in memory; `--output` writes private local
JSON files to the requested directory. The default `output/` directory is
gitignored. Do not use a tracked directory for real traffic data.

GitHub schedules may be delayed. Public repositories' schedules can be disabled
after 60 days without repository activity. Monitor the last success time and use
a manual run when needed; a gap beyond GitHub's 14-day window cannot be recovered
from the Traffic API alone.
