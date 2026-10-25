import assert from "node:assert/strict";
import test from "node:test";
import { installerDailyRows, installerPlatform, installerTotals, summarizeInstallerReleases } from "./downloads.mjs";

const asset = (name, downloadCount) => ({ name, downloadCount });
const release = (id, assets) => ({ id, tag: `v${id}`, publishedAt: `2026-10-${String(id).padStart(2, "0")}T00:00:00Z`, assets });
const totals = (windows, macos = 0, linux = 0, other = 0) => ({ total: windows + macos + linux + other, windows, macos, linux, other });
const snapshot = (date, installerDownloads, collectedAt = `${date}T03:17:00Z`) => ({ date, collectedAt, installerDownloads });

test("classifies only recognized primary installer extensions, case-insensitively", () => {
  for (const extension of ["exe", "msi", "msix", "msixbundle", "appx", "appxbundle"]) assert.equal(installerPlatform(`Hindsight.${extension.toUpperCase()}`), "windows");
  for (const extension of ["dmg", "pkg"]) assert.equal(installerPlatform(`Hindsight.${extension}`), "macos");
  for (const extension of ["deb", "rpm", "AppImage"]) assert.equal(installerPlatform(`Hindsight.${extension}`), "linux");
  assert.equal(installerPlatform("Hindsight.apk"), "other");
  for (const name of ["Hindsight.exe.sig", "Hindsight.dmg.sig", "latest.json", "Hindsight.zip", "Hindsight.app.tar.gz", "Hindsight", "setup.exe.sha256", "Hindsight.tar.gz", "exe", null]) {
    assert.equal(installerPlatform(name), null, String(name));
  }
});

test("aggregates installer platforms and excludes signature/manifest/archive download counts", () => {
  const data = { releases: [release(1, [asset("setup.exe", 10), asset("setup.exe.sig", 900), asset("latest.json", 1000), asset("mac.DMG", 20), asset("mac.zip", 300)]), release(2, [asset("linux.AppImage", 30), asset("linux.deb", 4), asset("android.apk", 5), asset("source.tar.gz", 800)])] };
  assert.deepEqual(installerTotals(data), totals(10, 20, 34, 5));
  assert.deepEqual(summarizeInstallerReleases(data), [
    { id: 1, tag: "v1", publishedAt: "2026-10-01T00:00:00Z", ...totals(10, 20) },
    { id: 2, tag: "v2", publishedAt: "2026-10-02T00:00:00Z", ...totals(0, 0, 34, 5) },
  ]);
});

test("keeps missing installer data unknown and successful empty/no-installer lists at zero", () => {
  assert.equal(installerTotals(null), null);
  assert.equal(installerTotals({}), null);
  assert.equal(installerTotals({ releaseDownloads: 2000 }), null);
  assert.equal(installerTotals({ releases: [{ id: 1 }] }), null);
  assert.deepEqual(installerTotals({ releases: [] }), totals(0));
  assert.deepEqual(installerTotals({ releases: [release(1, [asset("latest.json", 1000)])] }), totals(0));
  assert.equal(installerTotals({ releases: [release(1, [asset("setup.exe", null)])] }), null);
  assert.equal(installerTotals({ releases: [release(1, [asset("setup.exe", -1)])] }), null);
  assert.deepEqual(summarizeInstallerReleases({ releases: [release(1, [asset("setup.exe", 0)]), release(2, [asset("latest.json", 900)])] }), [{ id: 1, tag: "v1", publishedAt: "2026-10-01T00:00:00Z", ...totals(0) }]);
});

test("prefers valid preserved platform totals and falls back when aggregate validation fails", () => {
  const releases = [release(1, [asset("setup.exe", 5)])];
  assert.deepEqual(installerTotals({ installerDownloads: totals(12, 3), releases }), totals(12, 3));
  assert.deepEqual(installerTotals({ installerDownloads: { total: 9, windows: 5, macos: 0, linux: 0, other: 0 }, releases }), totals(5));
  assert.equal(installerTotals({ installerDownloads: { total: 9, windows: 5, macos: 0, linux: 0, other: 0 } }), null);
  assert.equal(installerTotals({ installerDownloads: { total: Number.MAX_SAFE_INTEGER, windows: Number.MAX_SAFE_INTEGER, macos: 1, linux: 0, other: 0 } }), null);
});

test("daily increases require adjacent UTC dates, known counts, and no reset", () => {
  const data = [
    snapshot("2026-10-20", totals(10, 2)),
    snapshot("2026-10-21", totals(13, 3)),
    snapshot("2026-10-23", totals(20, 3)),
    snapshot("2026-10-24", null),
    snapshot("2026-10-25", totals(30, 3)),
    snapshot("2026-10-26", totals(5, 1)),
    snapshot("2026-10-27", totals(7, 1)),
  ];
  assert.deepEqual(installerDailyRows(data), [
    { date: "2026-10-20", cumulative: 12, increase: null },
    { date: "2026-10-21", cumulative: 16, increase: 4 },
    { date: "2026-10-23", cumulative: 23, increase: null },
    { date: "2026-10-24", cumulative: null, increase: null },
    { date: "2026-10-25", cumulative: 33, increase: null },
    { date: "2026-10-26", cumulative: 6, increase: null },
    { date: "2026-10-27", cumulative: 8, increase: 2 },
  ]);
  assert.deepEqual(installerDailyRows(data, "macos").map((row) => row.increase), [null, 1, null, null, null, null, 0]);
});

test("sorts UTC dates, deduplicates by latest collection time, and accepts zero increases", () => {
  const data = [
    snapshot("2026-10-21", totals(8)),
    snapshot("2026-10-20", totals(6), "2026-10-20T06:00:00Z"),
    snapshot("2026-10-20", totals(3), "2026-10-20T02:00:00Z"),
    snapshot("2026-10-22", totals(8)),
    snapshot("2026-02-31", totals(100)),
    snapshot("invalid", totals(100)),
  ];
  assert.deepEqual(installerDailyRows(data), [
    { date: "2026-10-20", cumulative: 6, increase: null },
    { date: "2026-10-21", cumulative: 8, increase: 2 },
    { date: "2026-10-22", cumulative: 8, increase: 0 },
  ]);
  assert.deepEqual(installerDailyRows([]), []);
  assert.throws(() => installerDailyRows([], "android"), /Unknown installer platform/);
});
