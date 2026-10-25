import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const root = new URL("../", import.meta.url);

/** 应用主窗口的 CSP：指令 → 来源的集合。 */
function appCsp() {
  const conf = JSON.parse(readFileSync(new URL("src-tauri/tauri.conf.json", root), "utf8"));
  const csp = conf.app.security.csp;
  return Object.fromEntries(Object.entries(csp).map(([name, value]) => [name, value.split(/\s+/)]));
}

/** _headers 里给 path 的 Content-Security-Policy：指令 → 来源的集合。 */
function siteCsp(path) {
  const lines = readFileSync(new URL("web/_headers", root), "utf8").split("\n");
  const start = lines.indexOf(path);
  expect(start, `${path} is not in _headers`).toBeGreaterThanOrEqual(0);
  // 一个路径的响应头是它下面缩进的那几行，到下一个不缩进的行为止
  const block = [];
  for (const line of lines.slice(start + 1)) {
    if (!/^\s+\S/.test(line)) break;
    block.push(line.trim());
  }
  const header = block.find((line) => line.startsWith("Content-Security-Policy:"));
  expect(header, `${path} has no Content-Security-Policy`).toBeDefined();
  const value = header.slice("Content-Security-Policy:".length);
  return Object.fromEntries(
    value
      .split(";")
      .map((part) => part.trim().split(/\s+/))
      .filter((part) => part[0] !== "")
      .map(([name, ...sources]) => [name, sources]),
  );
}

// 只在 Tauri 里有的来源：网页上没有这些协议和主机
const TAURI_ONLY = new Set(["asset:", "http://asset.localhost"]);

describe("demo CSP", () => {
  // 主题的 CSS 能加载的图片和字体，预览里和应用里要一样
  it("the demo CSP matches the app's main window", () => {
    const app = appCsp();
    const demo = siteCsp("/demo/*");

    expect(new Set(demo["font-src"])).toEqual(new Set(app["font-src"]));
    // demo 的假数据要用 cdn.simpleicons.org 的图标，只多这一个
    expect(new Set(demo["img-src"])).toEqual(
      new Set([
        ...app["img-src"].filter((source) => !TAURI_ONLY.has(source)),
        "https://cdn.simpleicons.org",
      ]),
    );
  });
});
