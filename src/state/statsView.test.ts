// node 环境手工桩 localStorage（同 theme.test.ts；vitest 配置保持纯函数策略）。
// readInitial 在模块导入时按 scope 各读一次，所以用 resetModules + 动态 import 让
// 每个用例拿到一份「按当前存储值重新初始化」的 store——这正是切页往返 / 重启后
// 恢复各页视图走的路径。
import { beforeEach, describe, expect, it, vi } from "vitest";

const store = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
});

const key = (scope: string) => `hindsight.stats.view.${scope}`;

async function freshImport() {
  vi.resetModules();
  return import("./statsView");
}

describe("statsView", () => {
  beforeEach(() => {
    store.clear();
  });

  it("空存储 / 非法值：每个 scope 都回退 bars", async () => {
    store.set(key("week"), "donut");
    const m = await freshImport();
    expect(m.getStatsView("today")).toBe("bars");
    expect(m.getStatsView("week")).toBe("bars");
    expect(m.getStatsView("month")).toBe("bars");
  });

  it("已存合法值 → 各 scope 独立恢复（切页往返 / 重启保留每页上次选择）", async () => {
    store.set(key("today"), "pie");
    store.set(key("month"), "pie");
    const m = await freshImport();
    expect(m.getStatsView("today")).toBe("pie");
    expect(m.getStatsView("week")).toBe("bars"); // week 未存 → 不受 today 影响
    expect(m.getStatsView("month")).toBe("pie");
  });

  it("setStatsView 只改本 scope，不带动其它页", async () => {
    const m = await freshImport();
    m.setStatsView("today", "pie");
    expect(store.get(key("today"))).toBe("pie");
    expect(m.getStatsView("today")).toBe("pie");
    expect(m.getStatsView("week")).toBe("bars");
    expect(m.getStatsView("month")).toBe("bars");
    expect(store.get(key("week"))).toBeUndefined();
  });

  it("持久化 + 通知订阅者；同值不通知；退订后不再通知", async () => {
    const m = await freshImport();
    const seen: string[] = [];
    const off = m.subscribeStatsView(() => seen.push(m.getStatsView("today")));

    m.setStatsView("today", "pie");
    expect(m.getStatsView("today")).toBe("pie");
    expect(seen).toEqual(["pie"]);

    m.setStatsView("today", "pie"); // 同值：不写不通知
    expect(seen).toEqual(["pie"]);

    off();
    m.setStatsView("today", "bars"); // 退订后：值变了但不再通知
    expect(m.getStatsView("today")).toBe("bars");
    expect(seen).toEqual(["pie"]);
  });
});
