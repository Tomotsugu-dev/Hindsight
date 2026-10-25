import { describe, expect, it } from "vitest";
import { serveThemeData, themeDataSource } from "./themes-data.js";

const RAW = "https://raw.githubusercontent.com/Tomotsugu-dev/hindsight-plugins/stats/themes/";

describe("themeDataSource", () => {
  it("the worker forwards only index.json and <id>.json", () => {
    expect(themeDataSource("/themes/data/index.json")).toBe(`${RAW}index.json`);
    expect(themeDataSource("/themes/data/harbor.json")).toBe(`${RAW}harbor.json`);

    for (const path of [
      "/themes/data/",
      "/themes/data/Harbor.json",
      "/themes/data/harbor.css",
      "/themes/data/-harbor.json",
      "/themes/data/a/b.json",
      "/themes/data/..%2Fsecret.json",
      "/themes/data/harbor.json/x",
      "/themes/datax/harbor.json",
      "/themes/harbor.json",
    ]) {
      expect(themeDataSource(path), path).toBeNull();
    }
  });
});

/** 假的 fetch：记下请求的地址，返回 response。 */
function fakeFetch(response) {
  const calls = [];
  const fetch = async (url) => {
    calls.push(url);
    return response;
  };
  return { fetch, calls };
}

describe("serveThemeData", () => {
  it("passes the file through as JSON", async () => {
    const { fetch, calls } = fakeFetch(new Response('{"format_version":1,"themes":[]}'));

    const res = await serveThemeData(new Request("https://h.example/themes/data/index.json"), fetch);

    expect(calls).toEqual([`${RAW}index.json`]);
    expect(res.status).toBe(200);
    expect(res.headers.get("Content-Type")).toBe("application/json; charset=utf-8");
    expect(await res.text()).toBe('{"format_version":1,"themes":[]}');
  });

  // 下架的主题没有 <id>.json：照样是 404；raw 出错是 502；别的路径根本不去请求
  it("reports a missing file as 404 and other upstream errors as 502", async () => {
    const gone = fakeFetch(new Response("", { status: 404 }));
    const down = fakeFetch(new Response("", { status: 503 }));
    const other = fakeFetch(new Response("{}"));

    const missing = await serveThemeData(new Request("https://h.example/themes/data/x.json"), gone.fetch);
    const failed = await serveThemeData(new Request("https://h.example/themes/data/x.json"), down.fetch);
    const refused = await serveThemeData(new Request("https://h.example/themes/data/a.css"), other.fetch);

    expect(missing.status).toBe(404);
    expect(failed.status).toBe(502);
    expect(refused.status).toBe(404);
    expect(other.calls).toEqual([]);
  });

  it("answers only GET and HEAD", async () => {
    const { fetch, calls } = fakeFetch(new Response("{}"));

    const res = await serveThemeData(
      new Request("https://h.example/themes/data/index.json", { method: "POST" }),
      fetch,
    );

    expect(res.status).toBe(405);
    expect(calls).toEqual([]);
  });
});
