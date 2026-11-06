// 和 statsView.test.ts 一样手工桩 localStorage；存的值在模块导入时读一次，
// 所以每个用例 resetModules 后重新 import，走的是重启后恢复的那条路。
import { beforeEach, describe, expect, it, vi } from "vitest";

const store = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
});

const cardsKey = (scope: string) => `hindsight.stats.cards.${scope}`;
const modeKey = (scope: string) => `hindsight.stats.mode.${scope}`;

async function freshImport() {
  vi.resetModules();
  return import("./insightCards");
}

beforeEach(() => {
  store.clear();
  vi.restoreAllMocks();
});

describe("reading stored cards", () => {
  it("gives the defaults when nothing is stored", async () => {
    const m = await freshImport();
    expect(m.getCards("today")).toEqual(["vsPrev", "peak", "dominant"]);
    expect(m.getCards("week")).toEqual(["vsPrev", "avg", "avgVsPrev", "dominant"]);
    expect(m.getMode("week")).toBe("same");
  });

  it("drops unknown and repeated cards, keeps 6, fills up to 3 from the defaults", async () => {
    // 日统计没有 avg；peak 重复；只剩两张有效，从默认里补 vsPrev
    store.set(cardsKey("today"), JSON.stringify({ v: 1, cards: ["avg", "peak", "peak", "total"] }));
    const all = ["span", "avgSpan", "devices", "total", "dominant", "peak", "avg"];
    store.set(cardsKey("week"), JSON.stringify({ v: 1, cards: all }));
    const m = await freshImport();
    expect(m.getCards("today")).toEqual(["peak", "total", "vsPrev"]);
    expect(m.getCards("week")).toEqual(all.slice(0, 6));
  });

  it("gives the defaults and warns on another version or broken JSON", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    store.set(cardsKey("week"), JSON.stringify({ v: 2, cards: ["total", "span", "devices"] }));
    store.set(cardsKey("month"), "{not json");
    const m = await freshImport();
    expect(m.getCards("week")).toEqual(m.DEFAULT_CARDS.week);
    expect(m.getCards("month")).toEqual(m.DEFAULT_CARDS.month);
    expect(warn).toHaveBeenCalledTimes(2);
  });

  it("keeps each page's mode apart; the day page has none", async () => {
    store.set(modeKey("week"), "rolling");
    store.set(modeKey("today"), "rolling");
    const m = await freshImport();
    expect(m.getMode("week")).toBe("rolling");
    expect(m.getMode("month")).toBe("same");
    expect(m.getMode("today")).toBe("same");
  });
});

describe("editing cards", () => {
  it("refuses to go below 3 or above 6", async () => {
    const m = await freshImport();
    expect(m.removeCard("today", "peak")).toBe("min");
    expect(m.getCards("today")).toHaveLength(3);
    for (const id of ["total", "devices"] as const) expect(m.addCard("week", id)).toBeNull();
    expect(m.addCard("week", "span")).toBe("max");
    expect(m.getCards("week")).toHaveLength(6);
  });

  it("adds at a position, moves, and saves with the version", async () => {
    const m = await freshImport();
    m.addCard("week", "total", 1);
    expect(m.getCards("week")).toEqual(["vsPrev", "total", "avg", "avgVsPrev", "dominant"]);
    m.moveCard("week", "vsPrev", 4);
    expect(m.getCards("week")).toEqual(["total", "avg", "avgVsPrev", "dominant", "vsPrev"]);
    expect(JSON.parse(store.get(cardsKey("week"))!)).toEqual({
      v: 1,
      cards: ["total", "avg", "avgVsPrev", "dominant", "vsPrev"],
    });
    // 别的页不受影响，也不写存储
    expect(store.has(cardsKey("month"))).toBe(false);
  });

  it("does not add a card the page does not have", async () => {
    const m = await freshImport();
    expect(m.addCard("today", "avg")).toBeNull();
    expect(m.getCards("today")).toEqual(m.DEFAULT_CARDS.today);
  });

  it("puts back the defaults and keeps the mode", async () => {
    const m = await freshImport();
    m.setMode("week", "rolling");
    m.addCard("week", "span");
    m.resetCards("week");
    expect(m.getCards("week")).toEqual(m.DEFAULT_CARDS.week);
    expect(m.getMode("week")).toBe("rolling");
    expect(store.get(modeKey("week"))).toBe("rolling");
  });

  it("tells subscribers only when something changed", async () => {
    const m = await freshImport();
    const seen = vi.fn();
    const off = m.subscribeInsightCards(seen);
    m.setMode("month", "rolling");
    m.setMode("month", "rolling");
    m.moveCard("month", "vsPrev", 0);
    m.removeCard("today", "peak");
    m.addCard("month", "peak");
    expect(seen).toHaveBeenCalledTimes(2);
    off();
    m.resetCards("month");
    expect(seen).toHaveBeenCalledTimes(2);
  });
});
