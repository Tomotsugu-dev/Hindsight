import { describe, expect, it } from "vitest";
import { loadIndex } from "./index-data";

const HARBOR = {
  id: "harbor",
  name: "Harbor",
  author: "someone",
  description: { en: "Slate blue on cool grey." },
  version: "1.0.0",
  layout: "card",
  mode: "light",
  tokens: { "--color-accent": "#3b6e8f" },
  repo: "someone/hindsight-harbor",
  homepage: "https://github.com/someone/hindsight-harbor",
  release_url: "https://github.com/someone/hindsight-harbor/releases/tag/1.0.0",
  sha256: "ab12",
  downloads: 7,
};

/** 假的 fetch：返回 body（字符串原样返回，其余转成 JSON）和 status。 */
function respond(body: unknown, status = 200) {
  return (async () =>
    new Response(typeof body === "string" ? body : JSON.stringify(body), { status })) as typeof fetch;
}

describe("loadIndex", () => {
  it("returns the themes in the order of index.json", async () => {
    const nord = { ...HARBOR, id: "nord", downloads: 3 };

    const themes = await loadIndex(respond({ format_version: 1, themes: [HARBOR, nord] }));

    expect(themes.map((t) => t.id)).toEqual(["harbor", "nord"]);
  });

  it("reports the reason when index.json cannot be read or has an unknown format", async () => {
    await expect(loadIndex(respond("", 502))).rejects.toThrow("HTTP 502");
    await expect(loadIndex(respond("<html>"))).rejects.toThrow("not JSON");
    await expect(loadIndex(respond({ format_version: 2, themes: [] }))).rejects.toThrow("format 2");
    await expect(loadIndex(respond({ format_version: 1 }))).rejects.toThrow("no themes");
  });

  // 一条坏了只跳过它；链接不是 https 的也跳过，免得 javascript: 这类地址进到页面里
  it("skips an entry with a wrong field and keeps the others", async () => {
    const themes = await loadIndex(
      respond({
        format_version: 1,
        themes: [
          { ...HARBOR, id: "no-name", name: undefined },
          { ...HARBOR, id: "bad-mode", mode: "sepia" },
          { ...HARBOR, id: "script", homepage: "javascript:alert(1)" },
          HARBOR,
        ],
      }),
    );

    expect(themes.map((t) => t.id)).toEqual(["harbor"]);
  });
});
