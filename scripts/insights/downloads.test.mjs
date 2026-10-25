import assert from "node:assert/strict";
import test from "node:test";
import { installerDailyRows, installerPlatform, installerTotals, installerWindowSummary, summarizeInstallerReleases } from "./downloads.mjs";

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

const weekSnapshots = () => Array.from({ length: 8 }, (_, index) => snapshot(`2026-10-${18 + index}`, totals(1000 + index, 2000 + index * 2)));

test("seven-day total counts changes with the preceding baseline instead of lifetime downloads", () => {
  const summary = installerWindowSummary(weekSnapshots(), { endDate: "2026-10-25" });
  assert.equal(summary.days, 7);
  assert.equal(summary.startDate, "2026-10-19");
  assert.equal(summary.endDate, "2026-10-25");
  assert.deepEqual(summary.rows, Array.from({ length: 7 }, (_, index) => ({ date: `2026-10-${19 + index}`, downloads: 3 })));
  assert.equal(summary.knownDays, 7);
  assert.equal(summary.total, 21);
  assert.equal(summary.complete, true);
});

test("partial windows keep calendar gaps and first snapshots unknown rather than zero", () => {
  const summary = installerWindowSummary([
    snapshot("2026-10-20", totals(5)),
    snapshot("2026-10-21", totals(8)),
    snapshot("2026-10-23", totals(12)),
    snapshot("2026-10-24", totals(14)),
  ], { endDate: "2026-10-27" });
  assert.deepEqual(summary.rows, [
    { date: "2026-10-21", downloads: 3 },
    { date: "2026-10-22", downloads: null },
    { date: "2026-10-23", downloads: null },
    { date: "2026-10-24", downloads: 2 },
    { date: "2026-10-25", downloads: null },
    { date: "2026-10-26", downloads: null },
    { date: "2026-10-27", downloads: null },
  ]);
  assert.equal(summary.knownDays, 2);
  assert.equal(summary.total, 5);
  assert.equal(summary.complete, false);
  assert.deepEqual(installerWindowSummary([snapshot("2026-10-25", totals(9000))], { days: 1, endDate: "2026-10-25" }).rows, [{ date: "2026-10-25", downloads: null }]);
});

test("known zero downloads remain a complete zero while no coverage has a null total", () => {
  const unchanged = weekSnapshots().map((row) => ({ ...row, installerDownloads: totals(4) }));
  const summary = installerWindowSummary(unchanged, { endDate: "2026-10-25" });
  assert.equal(summary.total, 0);
  assert.equal(summary.knownDays, 7);
  assert.equal(summary.complete, true);
  assert.ok(summary.rows.every((row) => row.downloads === 0));
  const missing = installerWindowSummary([], { endDate: "2026-10-25" });
  assert.equal(missing.total, null);
  assert.equal(missing.knownDays, 0);
  assert.equal(missing.complete, false);
  assert.ok(missing.rows.every((row) => row.downloads === null));
});

test("counter resets do not invent negative or zero weekly downloads", () => {
  const summary = installerWindowSummary([
    snapshot("2026-10-20", totals(100)),
    snapshot("2026-10-21", totals(5)),
    snapshot("2026-10-22", totals(8)),
  ], { days: 2, endDate: "2026-10-22" });
  assert.deepEqual(summary.rows, [{ date: "2026-10-21", downloads: null }, { date: "2026-10-22", downloads: 3 }]);
  assert.equal(summary.total, 3);
  assert.equal(summary.knownDays, 1);
  assert.equal(summary.complete, false);
});

test("window excludes stale and future data and supports separate platform totals", () => {
  const data = [snapshot("2026-10-01", totals(100)), snapshot("2026-10-02", totals(900)), ...weekSnapshots(), snapshot("2026-10-26", totals(999999))];
  assert.equal(installerWindowSummary(data, { endDate: "2026-10-25" }).total, 21);
  assert.equal(installerWindowSummary(data, { endDate: "2026-10-25", platform: "windows" }).total, 7);
  assert.equal(installerWindowSummary(data, { endDate: "2026-10-25", platform: "macos" }).total, 14);
  const stale = installerWindowSummary(data.slice(0, 2), { endDate: "2026-10-25" });
  assert.equal(stale.total, null);
  assert.equal(stale.knownDays, 0);
});

test("window validates arguments and defaults its end date to today's UTC date", () => {
  for (const days of [0, -1, 367, 1.5, "7", null, NaN]) assert.throws(() => installerWindowSummary([], { days }), /days must be an integer/);
  for (const endDate of ["2026-02-31", "2026-10-25T00:00:00Z", "invalid", null]) assert.throws(() => installerWindowSummary([], { endDate }), /valid UTC date/);
  assert.throws(() => installerWindowSummary([], { platform: "android" }), /Unknown installer platform/);
  assert.equal(installerWindowSummary([], { days: 366, endDate: "2026-10-25" }).rows.length, 366);
  assert.equal(installerWindowSummary([]).endDate, new Date().toISOString().slice(0, 10));
});
