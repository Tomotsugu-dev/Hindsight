import { describe, expect, it } from "vitest";
import { NAV_ITEMS, ROUTES } from "@app/config/nav";
import { chunkKeysForPath, normalizeRoutePath } from "./routeChunks";

describe("demo routeChunks", () => {
  it("normalizeRoutePath 去掉 query、hash 和尾斜杠", () => {
    expect(normalizeRoutePath("/")).toBe("/");
    expect(normalizeRoutePath("/week/")).toBe("/week");
    expect(normalizeRoutePath("/settings/appearance/?tab=1#top")).toBe("/settings/appearance");
  });

  it("所有非首屏侧边栏一级路由都能映射到至少一个 chunk", () => {
    for (const item of NAV_ITEMS) {
      if (item.path === ROUTES.today) {
        expect(chunkKeysForPath(item.path)).toEqual([]);
      } else {
        expect(chunkKeysForPath(item.path).length, item.path).toBeGreaterThan(0);
      }
    }
  });

  it("带子 Tab 的页面同时预加载父页面与对应 Tab", () => {
    expect(chunkKeysForPath(ROUTES.history)).toEqual(["history", "historyStats"]);
    expect(chunkKeysForPath(`${ROUTES.history}/search`)).toEqual(["history", "search"]);
    expect(chunkKeysForPath(ROUTES.aiSummary)).toEqual(["aiSummary", "aiDaily"]);
    expect(chunkKeysForPath(`${ROUTES.aiSummary}/week`)).toEqual(["aiSummary", "aiWeekly"]);
    expect(chunkKeysForPath(ROUTES.categories)).toEqual(["categories", "categoriesList"]);
    expect(chunkKeysForPath(`${ROUTES.categories}/apps`)).toEqual(["categories", "apps"]);
    expect(chunkKeysForPath(ROUTES.apps)).toEqual(["categories", "apps"]);
    expect(chunkKeysForPath(ROUTES.settings)).toEqual(["settings", "settingsGeneral"]);
    expect(chunkKeysForPath(`${ROUTES.settings}/plugins`)).toEqual(["settings", "settingsPlugins"]);
  });

  it("未知路径返回空列表", () => {
    expect(chunkKeysForPath("/plugins/custom")).toEqual([]);
  });
});
