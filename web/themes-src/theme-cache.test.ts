import { beforeEach, describe, expect, it } from "vitest";
import { clearThemeCache, getCachedTheme, loadFullTheme } from "./theme-cache";

const HARBOR = {
  id: "harbor",
  layout: "card" as const,
  mode: "light" as const,
  tokens: { "--color-accent": "#3b6e8f" },
  css: ".custom { color: red; }",
};

function fakeFetch(body: unknown, status = 200) {
  let calls = 0;
  const fetchImpl = (async () => {
    calls += 1;
    return new Response(JSON.stringify(body), { status });
  }) as typeof fetch;
  return { fetchImpl, getCalls: () => calls };
}

describe("theme-cache", () => {
  beforeEach(() => {
    clearThemeCache();
  });

  it("caches loaded themes and deduplicates concurrent loads", async () => {
    const { fetchImpl, getCalls } = fakeFetch(HARBOR);

    const [a, b] = await Promise.all([
      loadFullTheme("harbor", fetchImpl),
      loadFullTheme("harbor", fetchImpl),
    ]);
    const c = await loadFullTheme("harbor", fetchImpl);

    expect(a).toEqual(HARBOR);
    expect(b).toEqual(HARBOR);
    expect(c).toEqual(HARBOR);
    expect(getCachedTheme("harbor")).toEqual(HARBOR);
    expect(getCalls()).toBe(1);
  });

  it("does not cache failures so a retry can succeed", async () => {
    const failed = fakeFetch(HARBOR, 502);
    await expect(loadFullTheme("harbor", failed.fetchImpl)).rejects.toThrow();
    expect(getCachedTheme("harbor")).toBeNull();

    const ok = fakeFetch(HARBOR, 200);
    await expect(loadFullTheme("harbor", ok.fetchImpl)).resolves.toEqual(HARBOR);
    expect(getCachedTheme("harbor")).toEqual(HARBOR);
  });
});
