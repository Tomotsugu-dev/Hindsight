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
