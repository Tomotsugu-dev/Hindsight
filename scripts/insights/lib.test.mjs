import assert from "node:assert/strict";
import test from "node:test";
import { collectAll, createJsonClient, historyKey, mergeByDate, mergeHistory, normalizeTrafficDays, snapshotKey, validateConfig } from "./lib.mjs";
import { memoryStore } from "./collect.mjs";

const repository = "Tomotsugu-dev/Hindsight";
const sample = (date, count = 4) => ({
  version: 1,
  repository,
  collectedAt: `${date}T03:17:00.000Z`,
  views: [{ date, count, uniques: Math.min(count, 2) }],
  clones: [{ date, count: 1, uniques: 1 }],
  snapshot: { date, collectedAt: `${date}T03:17:00.000Z`, stars: 3, forks: 1, watchers: 1, openIssues: 2, trafficWindow: { views: { count, uniques: 2 }, clones: { count: 1, uniques: 1 } }, referrers: [], paths: [], releases: [] },
});

test("overlapping daily windows replace dates, preserve old days, and remain idempotent", () => {
  const old = [ { date: "2026-10-01", count: 10, uniques: 2 }, { date: "2026-10-20", count: 3, uniques: 1 } ];
  const incoming = [ { date: "2026-10-20", count: 8, uniques: 3 }, { date: "2026-10-21", count: 0, uniques: 0 } ];
  const merged = mergeByDate(old, incoming);
  assert.deepEqual(merged, [old[0], incoming[0], incoming[1]]);
  assert.deepEqual(mergeByDate(merged, incoming), merged);
});

test("missing days stay missing, while explicit zeros replace previous counts", () => {
  const old = [{ date: "2026-10-20", count: 5, uniques: 3 }];
  assert.deepEqual(mergeByDate(old, []), old);
  assert.deepEqual(mergeByDate(old, [{ date: "2026-10-20", count: 0, uniques: 0 }]), [{ date: "2026-10-20", count: 0, uniques: 0 }]);
  assert.throws(() => normalizeTrafficDays(undefined), /Missing/);
  assert.throws(() => normalizeTrafficDays([{ timestamp: "2026-10-20T00:00:00Z", count: null, uniques: 0 }]), /Invalid/);
});

test("rerunning on the same UTC date replaces its snapshot without inflating history", () => {
  const first = mergeHistory(null, sample("2026-10-20"));
  const next = mergeHistory(first, sample("2026-10-20", 8));
  assert.equal(next.firstCollectedAt, first.firstCollectedAt);
  assert.equal(next.views.length, 1);
  assert.equal(next.views[0].count, 8);
  assert.equal(next.snapshots.length, 1);
});

test("release totals survive detail compaction and only the latest two UTC dates retain assets", () => {
  let history = null;
  for (const [date, downloads] of [["2026-10-20", 10], ["2026-10-21", 15], ["2026-10-22", 21], ["2026-10-23", 28]]) {
    const collected = sample(date);
    collected.snapshot.releases = [{ id: 1, assets: [{ id: 2, downloadCount: downloads }] }];
    history = mergeHistory(history, collected);
  }
  assert.deepEqual(history.snapshots.map((snapshot) => snapshot.releaseDownloads), [10, 15, 21, 28]);
  assert.deepEqual(history.snapshots.filter((snapshot) => Object.hasOwn(snapshot, "releases")).map((snapshot) => snapshot.date), ["2026-10-22", "2026-10-23"]);
  const rerun = sample("2026-10-23");
  rerun.snapshot.releases = [{ id: 1, assets: [{ id: 2, downloadCount: 28 }] }];
  assert.deepEqual(mergeHistory(history, rerun), history);
});

test("legacy release histories migrate totals without confusing missing data with zero", () => {
  const legacy = mergeHistory(null, sample("2026-10-20"));
  legacy.snapshots = [
    { date: "2026-10-20", releases: [{ assets: [{ downloadCount: 4 }, { downloadCount: 6 }] }] },
    { date: "2026-10-21", releases: [] },
    { date: "2026-10-22" },
    { date: "2026-10-23", releaseDownloads: 12 },
    { date: "2026-10-24", releases: [{ assets: [{ downloadCount: null }] }] },
  ];
  const merged = mergeHistory(legacy, sample("2026-10-25"));
  assert.deepEqual(merged.snapshots.map((snapshot) => snapshot.releaseDownloads), [10, 0, null, 12, null, 0]);
  assert.equal(Object.hasOwn(merged.snapshots[0], "releases"), false);
  assert.equal(Object.hasOwn(merged.snapshots[4], "releases"), true);
  assert.deepEqual(merged.snapshots[5].releases, []);
});

test("installer platform totals migrate before compaction and survive later collections", () => {
  const legacy = mergeHistory(null, sample("2026-10-20"));
  legacy.snapshots = [
    { date: "2026-10-20", releases: [{ assets: [{ name: "setup.exe", downloadCount: 4 }, { name: "app.dmg", downloadCount: 6 }, { name: "setup.exe.sig", downloadCount: 900 }] }] },
    { date: "2026-10-21", releases: [] },
    { date: "2026-10-22", releaseDownloads: 4000 },
  ];
  const incoming = sample("2026-10-23");
  incoming.snapshot.releases = [{ assets: [{ name: "setup.exe", downloadCount: 8 }, { name: "app.dmg", downloadCount: 10 }] }];
  const history = mergeHistory(legacy, incoming);
  assert.deepEqual(history.snapshots.map((snapshot) => snapshot.installerDownloads), [
    { total: 10, windows: 4, macos: 6, linux: 0, other: 0 },
    { total: 0, windows: 0, macos: 0, linux: 0, other: 0 },
    null,
    { total: 18, windows: 8, macos: 10, linux: 0, other: 0 },
  ]);
  assert.equal(Object.hasOwn(history.snapshots[0], "releases"), false);
  const later = mergeHistory(history, sample("2026-10-24"));
  assert.deepEqual(later.snapshots[0].installerDownloads, { total: 10, windows: 4, macos: 6, linux: 0, other: 0 });
  assert.deepEqual(mergeHistory(history, incoming), history);
  assert.equal(incoming.snapshot.releases[0].assets[0].downloadCount, 8);
});

test("daily recovery copies retain full release catalogs after history compaction", async () => {
  const store = memoryStore();
  for (const date of ["2026-10-20", "2026-10-21", "2026-10-22"]) {
    const collected = sample(date);
    collected.snapshot.releases = [{ id: 1, assets: [{ id: 2, downloadCount: 10 }] }];
    await collectAll({ repositories: [repository], store, collectRepository: async () => collected, now: new Date(collected.collectedAt) });
    // Compaction must not mutate the original response used for recovery.
    assert.equal(collected.snapshot.releases[0].assets[0].downloadCount, 10);
  }
  const history = await store.get(historyKey(repository));
  assert.equal(Object.hasOwn(history.snapshots[0], "releases"), false);
  const recovery = await store.get(snapshotKey(repository, "2026-10-20"));
  assert.equal(recovery.snapshot.releases[0].assets[0].downloadCount, 10);
});

test("a failed GitHub request preserves successful history and reports stale collection", async () => {
  const store = memoryStore();
  await collectAll({ repositories: [repository], store, collectRepository: async () => sample("2026-10-20"), now: new Date("2026-10-20T03:17:00Z") });
  const history = await store.get(historyKey(repository));
  const previousStatus = await store.get("status:v1");
  const result = await collectAll({ repositories: [repository], store, collectRepository: async () => { throw new Error("HTTP 403 secret-value"); }, now: new Date("2026-10-21T03:17:00Z"), secrets: ["secret-value"] });
  assert.deepEqual(await store.get(historyKey(repository)), history);
  assert.equal(await store.get(snapshotKey(repository, "2026-10-21")), null);
  assert.equal(result.status.lastSuccessAt, previousStatus.lastSuccessAt);
  assert.equal(result.status.repositories[0].lastSuccessAt, previousStatus.repositories[0].lastSuccessAt);
  assert.equal(result.status.errors[0].message, "HTTP 403 [redacted]");
});

test("a storage failure does not claim a successful history update", async () => {
  const backing = memoryStore();
  const store = { ...backing, put: async (key, value) => {
    if (key.startsWith("history:")) throw new Error("KV unavailable");
    return backing.put(key, value);
  } };
  const { status } = await collectAll({ repositories: [repository], store, collectRepository: async () => sample("2026-10-20"), now: new Date("2026-10-20T03:17:00Z") });
  assert.equal(status.lastSuccessAt, null);
  assert.equal(status.repositories[0].lastSuccessAt, null);
  assert.equal(status.errors.length, 1);
  assert.ok(await backing.get(snapshotKey(repository, "2026-10-20")));
  assert.equal(await backing.get(historyKey(repository)), null);
});

test("retry temporary failures but do not silently retry forbidden Traffic as zero", async () => {
  let calls = 0;
  const pauses = [];
  const request = createJsonClient({
    fetchImpl: async () => ++calls === 1 ? new Response("temporary", { status: 503 }) : Response.json({ count: 4 }),
    sleep: async (delay) => { pauses.push(delay); },
  });
  assert.deepEqual(await request("https://api.github.com/repos/owner/repo/traffic/views"), { count: 4 });
  assert.equal(calls, 2);
  assert.deepEqual(pauses, [1000]);
  const forbidden = createJsonClient({ fetchImpl: async () => new Response("forbidden", { status: 403 }), sleep: async () => assert.fail("Unexpected retry") });
  await assert.rejects(forbidden("https://api.github.com/repos/owner/repo/traffic/views"), /HTTP 403/);
});

test("config prevents namespace placeholders in production and repository aliases", () => {
  assert.throws(() => validateConfig({ version: 1, repositories: [repository], kvNamespaceId: "PLACEHOLDER" }), /kvNamespaceId/);
  assert.doesNotThrow(() => validateConfig({ version: 1, repositories: [repository] }, { requireNamespace: false }));
  assert.throws(() => validateConfig({ version: 1, repositories: [repository, repository.toLowerCase()] }, { requireNamespace: false }), /duplicate/);
});
