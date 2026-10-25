import { describe, expect, it } from "vitest";
import { fetchPreviewTheme, previewThemeId } from "./preview-theme";

describe("previewThemeId", () => {
  it("reads the theme id from ?theme=", () => {
    expect(previewThemeId("?theme=harbor")).toBe("harbor");
    expect(previewThemeId("?lang=ja&theme=harbor")).toBe("harbor");

    // 值不合 id 的规则：不拿它去拼地址
    for (const search of ["", "?lang=ja", "?theme=", "?theme=Harbor", "?theme=..%2Fx", "?theme=-a"]) {
      expect(previewThemeId(search), search).toBeNull();
    }
  });
});

const HARBOR = {
  id: "harbor",
  layout: "card",
  mode: "light",
  tokens: { "--color-accent": "#3b6e8f" },
  css: null,
};

/** 假的 fetch：记下请求的地址，返回 body 和 status。 */
function fakeFetch(body: unknown, status = 200) {
  const calls: string[] = [];
  const fetchImpl = (async (input: RequestInfo | URL) => {
    calls.push(String(input));
    return new Response(JSON.stringify(body), { status });
  }) as typeof fetch;
  return { fetchImpl, calls };
}

describe("fetchPreviewTheme", () => {
  it("reads /themes/data/<id>.json", async () => {
    const { fetchImpl, calls } = fakeFetch(HARBOR);

    await expect(fetchPreviewTheme("harbor", fetchImpl)).resolves.toEqual(HARBOR);
    expect(calls).toEqual(["/themes/data/harbor.json"]);
  });

  // 下架了（404）、文件不是主题、读到的是别的主题：都当作读不到
  it("rejects a missing file, a file that is not a theme, and another theme", async () => {
    const cases: [unknown, number][] = [
      [HARBOR, 404],
      [{ id: "harbor" }, 200],
      [{ ...HARBOR, id: "nord" }, 200],
    ];
    for (const [body, status] of cases) {
      await expect(fetchPreviewTheme("harbor", fakeFetch(body, status).fetchImpl)).rejects.toThrow();
    }
  });
});
