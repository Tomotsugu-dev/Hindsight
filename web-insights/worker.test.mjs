import assert from "node:assert/strict";
import test from "node:test";
import { BASE_PATH, createHandler } from "./handler.mjs";

const repository = "Tomotsugu-dev/Hindsight";
const handler = createHandler({ html: "<h1>Private dashboard</h1>", css: "body{}", javascript: "console.log('private')", repositories: [repository] });
const authenticated = { access: { aud: "test-access-audience" } };
const env = { ACCESS_AUD: "test-access-audience" };
const request = (path = "/", options) => new Request(`https://hindsight.kyosweb.com${BASE_PATH}${path}`, options);

test("all dashboard resources reject unauthenticated and forged-header requests before reading storage", async () => {
  let reads = 0;
  const bindings = { ...env, INSIGHTS: { get: async () => { reads++; return {}; } } };
  for (const path of ["", "/", "/dashboard.css", "/dashboard.js", "/api/status", `/api/repository?repo=${repository}`]) {
    const res = await handler.fetch(request(path, { headers: { "Cf-Access-Jwt-Assertion": "forged", "Cf-Access-Authenticated-User-Email": "admin@example.com" } }), bindings, {});
    assert.equal(res.status, 403, path);
    assert.equal(res.headers.get("Cache-Control"), "private, no-store");
  }
  assert.equal(reads, 0);
});

test("a missing or different Access audience remains closed", async () => {
  assert.equal((await handler.fetch(request(), env, { access: { aud: "different-app" } })).status, 403);
  assert.equal((await handler.fetch(request(), {}, authenticated)).status, 403);
});

test("alternate worker hostnames do not expose the dashboard", async () => {
  assert.equal((await handler.fetch(new Request(`https://hindsight-insights.example.workers.dev${BASE_PATH}/`), env, authenticated)).status, 404);
});

test("authenticated static files and canonical redirect have private cache and CSP headers", async () => {
  const res = await handler.fetch(request(), env, authenticated);
  assert.equal(res.status, 200);
  assert.match(await res.text(), /Private dashboard/);
  assert.match(res.headers.get("Content-Security-Policy"), /script-src 'self'/);
  assert.equal(res.headers.get("X-Robots-Tag"), "noindex, nofollow, noarchive");
  const redirect = await handler.fetch(request(""), env, authenticated);
  assert.equal(redirect.status, 308);
  assert.equal(redirect.headers.get("Location"), `${BASE_PATH}/`);
});

test("repository reads are restricted to the configured list", async () => {
  const reads = [];
  const bindings = { ...env, INSIGHTS: { get: async (key) => { reads.push(key); return { repository }; } } };
  const unknown = await handler.fetch(request("/api/repository?repo=other/private"), bindings, authenticated);
  assert.equal(unknown.status, 404);
  assert.deepEqual(reads, []);
  const known = await handler.fetch(request(`/api/repository?repo=${repository}`), bindings, authenticated);
  assert.equal(known.status, 200);
  assert.deepEqual(reads, [`history:v1:${repository}`]);
  assert.deepEqual(await known.json(), { repository });
});

test("first-run status shows configured repositories without inventing successful collection", async () => {
  const res = await handler.fetch(request("/api/status"), { ...env, INSIGHTS: { get: async () => null } }, authenticated);
  const status = await res.json();
  assert.equal(status.lastSuccessAt, null);
  assert.equal(status.repositories[0].repository, repository);
  assert.equal(status.repositories[0].lastSuccessAt, null);
});

test("storage failure returns a generic error instead of exception details", async () => {
  const res = await handler.fetch(request("/api/status"), { ...env, INSIGHTS: { get: async () => { throw new Error("secret storage details"); } } }, authenticated);
  assert.equal(res.status, 503);
  assert.doesNotMatch(await res.text(), /secret storage details/);
});

test("authenticated requests cannot mutate storage", async () => {
  const res = await handler.fetch(request("/api/status", { method: "POST" }), env, authenticated);
  assert.equal(res.status, 405);
  assert.equal(res.headers.get("Allow"), "GET, HEAD");
});

test("HEAD omits bodies for authenticated pages, API data, and redirects", async () => {
  const bindings = { ...env, INSIGHTS: { get: async () => ({ repository }) } };
  for (const [path, status] of [["/", 200], ["/dashboard.css", 200], ["/dashboard.js", 200], ["/api/status", 200], [`/api/repository?repo=${repository}`, 200], ["", 308]]) {
    const get = await handler.fetch(request(path), bindings, authenticated);
    const head = await handler.fetch(request(path, { method: "HEAD" }), bindings, authenticated);
    assert.equal(head.status, status, path);
    assert.deepEqual([...head.headers], [...get.headers], path);
    assert.equal(head.body, null, path);
    assert.equal(await head.text(), "", path);
  }
});

test("HEAD error replies preserve authentication and storage boundaries without bodies", async () => {
  let unauthenticatedReads = 0;
  const cases = [
    { path: "/api/status", bindings: { ...env, INSIGHTS: { get: async () => { unauthenticatedReads++; return {}; } } }, context: {}, status: 403 },
    { path: "/api/status", bindings: env, context: { access: { aud: "different-app" } }, status: 403 },
    { path: "/missing", bindings: env, context: authenticated, status: 503 },
    { path: "/api/repository?repo=other/private", bindings: { ...env, INSIGHTS: { get: async () => null } }, context: authenticated, status: 404 },
    { path: `/api/repository?repo=${repository}`, bindings: { ...env, INSIGHTS: { get: async () => null } }, context: authenticated, status: 404 },
    { path: "/api/status", bindings: { ...env, INSIGHTS: { get: async () => { throw new Error("private storage details"); } } }, context: authenticated, status: 503 },
  ];
  for (const { path, bindings, context, status } of cases) {
    const head = await handler.fetch(request(path, { method: "HEAD", headers: { "Cf-Access-Jwt-Assertion": "forged" } }), bindings, context);
    assert.equal(head.status, status, path);
    assert.equal(head.body, null, path);
    assert.equal(await head.text(), "", path);
    assert.equal(head.headers.get("Cache-Control"), "private, no-store", path);
  }
  assert.equal(unauthenticatedReads, 0);
  const alternateHost = await handler.fetch(new Request(`https://hindsight-insights.example.workers.dev${BASE_PATH}/`, { method: "HEAD" }), env, authenticated);
  assert.equal(alternateHost.status, 404);
  assert.equal(alternateHost.body, null);
});
