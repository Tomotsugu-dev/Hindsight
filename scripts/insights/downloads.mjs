// Shared by the collector and dashboard. Keep this module browser-compatible:
// it has no Node imports, filesystem access, or third-party dependencies.
const PLATFORMS = ["windows", "macos", "linux", "other"];
const DAY_MS = 24 * 60 * 60 * 1000;

export function installerPlatform(name) {
  if (typeof name !== "string") return null;
  if (/\.(exe|msi|msix|msixbundle|appx|appxbundle)$/i.test(name)) return "windows";
  if (/\.(dmg|pkg)$/i.test(name)) return "macos";
  if (/\.(deb|rpm|appimage)$/i.test(name)) return "linux";
  if (/\.apk$/i.test(name)) return "other";
  return null;
}

const validCount = (value) => Number.isSafeInteger(value) && value >= 0;
const emptyTotals = () => ({ total: 0, windows: 0, macos: 0, linux: 0, other: 0 });

function validatedTotals(value) {
  if (!value || !validCount(value.total) || PLATFORMS.some((platform) => !validCount(value[platform]))) return null;
  const sum = PLATFORMS.reduce((total, platform) => total + value[platform], 0);
  if (!Number.isSafeInteger(sum) || sum !== value.total) return null;
  return { total: value.total, windows: value.windows, macos: value.macos, linux: value.linux, other: value.other };
}

function summarizeAssets(assets) {
  if (!Array.isArray(assets)) return null;
  const totals = emptyTotals();
  let installers = 0;
  for (const asset of assets) {
    // A missing name could hide an installer, whereas a named unknown file
    // (including signatures, manifests, and archives) is deliberately excluded.
    if (typeof asset?.name !== "string") return null;
    const platform = installerPlatform(asset.name);
    if (!platform) continue;
    if (!validCount(asset.downloadCount)) return null;
    installers++;
    totals[platform] += asset.downloadCount;
    totals.total += asset.downloadCount;
    if (!validCount(totals[platform]) || !validCount(totals.total)) return null;
  }
  return { totals, installers };
}

export function installerTotals(snapshot) {
  const preserved = validatedTotals(snapshot?.installerDownloads);
  if (preserved) return preserved;
  if (!Array.isArray(snapshot?.releases)) return null;
  const totals = emptyTotals();
  for (const release of snapshot.releases) {
    const summary = summarizeAssets(release?.assets);
    if (!summary) return null;
    for (const field of ["total", ...PLATFORMS]) {
      totals[field] += summary.totals[field];
      if (!validCount(totals[field])) return null;
    }
  }
  return totals;
}

export function summarizeInstallerReleases(snapshot) {
  if (!Array.isArray(snapshot?.releases)) return [];
  return snapshot.releases.flatMap((release) => {
    const summary = summarizeAssets(release?.assets);
    if (!summary || !summary.installers) return [];
    return [{ id: release.id, tag: release.tag, publishedAt: release.publishedAt, ...summary.totals }];
  });
}

function utcDay(date) {
  if (typeof date !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(date)) return null;
  const timestamp = Date.parse(`${date}T00:00:00.000Z`);
  if (!Number.isFinite(timestamp) || new Date(timestamp).toISOString().slice(0, 10) !== date) return null;
  return timestamp;
}

function collectionTime(snapshot) {
  const timestamp = typeof snapshot?.collectedAt === "string" ? Date.parse(snapshot.collectedAt) : NaN;
  return Number.isFinite(timestamp) ? timestamp : -Infinity;
}

export function installerDailyRows(snapshots, platform = "total") {
  if (platform !== "total" && !PLATFORMS.includes(platform)) throw new RangeError("Unknown installer platform");
  const byDate = new Map();
  for (const snapshot of Array.isArray(snapshots) ? snapshots : []) {
    if (utcDay(snapshot?.date) === null) continue;
    const previous = byDate.get(snapshot.date);
    // Keep the last entry on equal timestamps, including legacy records with
    // no collection timestamp. Newer collection timestamps always win.
    if (!previous || collectionTime(snapshot) >= collectionTime(previous)) byDate.set(snapshot.date, snapshot);
  }
  const ordered = [...byDate.values()].sort((a, b) => a.date.localeCompare(b.date));
  let previous = null;
  return ordered.map((snapshot) => {
    const cumulative = installerTotals(snapshot)?.[platform] ?? null;
    const day = utcDay(snapshot.date);
    const increase = previous && day - previous.day === DAY_MS && cumulative !== null && previous.cumulative !== null && cumulative >= previous.cumulative
      ? cumulative - previous.cumulative
      : null;
    previous = { day, cumulative };
    return { date: snapshot.date, cumulative, increase };
  });
}
