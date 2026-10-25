import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

const buildSource = readFileSync(fileURLToPath(new URL("./build.mjs", import.meta.url)), "utf8");

test("public site build preserves public pages and omits the internal dashboard tree", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "hindsight-public-build-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const write = (relative, contents) => {
    const target = path.join(root, relative);
    mkdirSync(path.dirname(target), { recursive: true });
    writeFileSync(target, contents);
  };
  const publicFiles = new Map([
    ["index.html", '<!doctype html><link href="/assets/site.css"><a href="/demo/">Demo</a><a href="/docs/">Docs</a><video src="/assets/video/fixture.mp4" poster="/assets/video/fixture.jpg"></video>'],
    ["assets/site.css", "body { color: #123; }"],
    ["demo/index.html", "<!doctype html><title>Public demo</title>"],
    ["themes/index.html", "<!doctype html><title>Public themes</title>"],
    ["docs/index.html", "<!doctype html><title>Public documentation</title>"],
  ]);
  for (const [relative, contents] of publicFiles) write(`web/${relative}`, contents);
  write("docs/intro/en/shot_video/fixture.mp4", "synthetic video fixture");
  write("docs/intro/en/shot_video/fixture.jpg", "synthetic poster fixture");
  write("web/internal/github-insights/dashboard.html", "<!doctype html><title>Internal dashboard marker</title>");
  write("web/internal/github-insights/worker.mjs", 'export default { fetch: () => new Response("Internal worker marker") };');
  write("web/internal/github-insights/nested/private-marker.txt", "synthetic internal-only marker");
  write("web/worker.js", "source-only public Worker");
  write("web/themes-data.js", "source-only Worker module");
  write("web/headers.test.js", "test only");
  write("web/wrangler.jsonc", "{}");
  write("scripts/web/build.mjs", buildSource);

  const result = spawnSync(process.execPath, ["scripts/web/build.mjs"], { cwd: root, encoding: "utf8" });
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  for (const [relative, contents] of publicFiles) {
    assert.equal(readFileSync(path.join(root, "dist-web", relative), "utf8"), contents, relative);
  }
  assert.equal(readFileSync(path.join(root, "dist-web/assets/video/fixture.mp4"), "utf8"), "synthetic video fixture");
  assert.equal(readFileSync(path.join(root, "dist-web/assets/video/fixture.jpg"), "utf8"), "synthetic poster fixture");
  assert.equal(existsSync(path.join(root, "dist-web/internal")), false, "internal tools must not be published with the public site");
  assert.equal(existsSync(path.join(root, "dist-web/worker.js")), false);
  assert.equal(existsSync(path.join(root, "dist-web/themes-data.js")), false);
  assert.equal(existsSync(path.join(root, "dist-web/headers.test.js")), false);
  assert.equal(existsSync(path.join(root, "dist-web/wrangler.jsonc")), false);
});
