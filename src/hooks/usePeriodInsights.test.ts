import { describe, expect, it } from "vitest";
import type { BreakdownSlice } from "./useSuperCategoryBreakdown";
import { drillMinutes, insightDrill } from "./usePeriodInsights";

const slice = (id: string, cats: [string, number][]): BreakdownSlice => ({
  id,
  name: id,
  color: "#000",
  icon: "",
  minutes: cats.reduce((sum, [, m]) => sum + m, 0),
  cats: cats.map(([catId, minutes]) => ({ id: catId, name: catId, color: "#000", minutes })),
});

// 本月「娱乐」= 游戏 68 + 影音 32；上月 = 游戏 50 + 影音 40
const curr = [
  slice("work", [["code", 200]]),
  slice("fun", [
    ["game", 68],
    ["video", 32],
  ]),
];
const prev = [
  slice("fun", [
    ["game", 50],
    ["video", 40],
  ]),
];

describe("insightDrill", () => {
  it("选中小类：卡片只算这个小类，上期按同一个小类取，没有构成卡片", () => {
    const drill = insightDrill(curr, prev, "fun", "video");
    expect(drill).toEqual({
      catIds: new Set(["video"]),
      minutes: 32,
      prevMinutes: 40,
      top: null,
    });
  });

  it("只点进大类：卡片算整个大类，构成卡片是大类里最多的小类", () => {
    const drill = insightDrill(curr, prev, "fun", null);
    expect(drill?.catIds).toEqual(new Set(["game", "video"]));
    expect(drill?.minutes).toBe(100);
    expect(drill?.prevMinutes).toBe(90);
    expect(drill?.top?.id).toBe("game");
  });

  it("选中的小类这期没有时间：退回大类", () => {
    expect(insightDrill(curr, prev, "fun", "music")?.minutes).toBe(100);
  });

  it("都没选：整期，undefined", () => {
    expect(insightDrill(curr, prev, null, null)).toBeUndefined();
  });
});

describe("drillMinutes", () => {
  it("小类优先于大类，上期没有按 0 算", () => {
    expect(drillMinutes(curr, "fun", "video")).toBe(32);
    expect(drillMinutes(curr, "fun", null)).toBe(100);
    expect(drillMinutes(prev, "work", null)).toBe(0);
  });
});
