// 用真的中文文案跑一个独立的 i18next 实例，断言写成用户看到的样子
import i18next, { type TFunction } from "i18next";
import { beforeAll, describe, expect, it } from "vitest";
import type { InsightFocus } from "../../hooks/useInsightData";
import zhCN from "../../i18n/locales/zh-CN.json";
import { insightWindows } from "../../utils/insightWindows";
import { cardContext, cardLabel, scopeNote } from "./cardText";

let t: TFunction;
beforeAll(async () => {
  const i18n = i18next.createInstance();
  await i18n.init({ lng: "zh-CN", resources: { "zh-CN": { translation: zhCN } } });
  t = i18n.t;
});

// 2026-11-05 周四 15:40
const thu = new Date(2026, 10, 5, 15, 40);
const all = { kind: "all" } as const;
const note = (
  scope: "today" | "week" | "month",
  offset: number,
  mode: "same" | "rolling",
  now = thu,
) => {
  const w = insightWindows(scope, offset, mode, now);
  return scopeNote(t, cardContext(scope, mode, w, now, all), w, now);
};

describe("scopeNote", () => {
  it("says up to which minute the previous period counts", () => {
    expect(note("today", 0, "same")).toBe("今天 0:00 到现在，对比昨天 0:00 到 15:40");
    expect(note("week", 0, "same")).toBe("本周一 0:00 到现在，对比上周一 0:00 到上周四 15:40");
    expect(note("month", 0, "same")).toBe(
      "11月1日 0:00 到现在，对比 10月1日 0:00 到 10月5日 15:40",
    );
  });

  it("says when the whole previous month counts", () => {
    expect(note("month", 0, "same", new Date(2026, 2, 30, 9, 0))).toBe(
      "3月1日 0:00 到现在，对比 2月1日 – 2月28日（上月没有今天这个日子，算整月）",
    );
  });

  it("names both whole periods for a past one", () => {
    expect(note("week", -1, "same")).toBe("10月26日 – 11月1日，对比 10月19日 – 10月25日");
    expect(note("today", -1, "same")).toBe("11月4日，对比 11月3日");
  });

  it("names the rolling days, and leaves out 'recent' for a past period", () => {
    expect(note("week", 0, "rolling")).toBe(
      "近 7 天：10月29日 – 11月4日（不含今天），对比前 7 天：10月22日 – 10月28日",
    );
    expect(note("week", -2, "rolling")).toBe(
      "10月19日 – 10月25日 这 7 天，对比前 7 天：10月12日 – 10月18日",
    );
  });
});

describe("cardLabel", () => {
  const minutes = (m: number) => `${m}m`;
  const ctx = (
    scope: "today" | "week",
    offset: number,
    mode: "same" | "rolling",
    focus: InsightFocus = all,
  ) => {
    const w = insightWindows(scope, offset, mode, thu);
    return cardContext(scope, mode, w, thu, focus);
  };

  it("says 'so far' only while the period is still going", () => {
    expect(cardLabel(t, "vsPrev", ctx("week", 0, "same"), null, minutes)).toBe("较上周同期");
    expect(cardLabel(t, "vsPrev", ctx("week", -1, "same"), null, minutes)).toBe("较上周");
    expect(cardLabel(t, "vsPrev", ctx("week", 0, "rolling"), null, minutes)).toBe("较前 7 天");
    expect(cardLabel(t, "vsPrev", ctx("today", 0, "same"), null, minutes)).toBe("较昨日同期");
  });

  it("names what the dominant card shows for each focus", () => {
    const label = (focus: InsightFocus) =>
      cardLabel(t, "dominant", ctx("week", 0, "same", focus), null, minutes);
    expect(label(all)).toBe("主力");
    expect(label({ kind: "super", superId: "work" })).toBe("构成");
    expect(label({ kind: "app", groupId: "code", categoryId: "work" })).toBe("占所属分类");
  });

  it("adds the peak's time", () => {
    const week = ctx("week", 0, "same");
    expect(
      cardLabel(t, "peak", week, { kind: "peakDay", date: "2026-11-03", minutes: 280 }, minutes),
    ).toBe("峰值 · 280m");
    expect(cardLabel(t, "peak", week, null, minutes)).toBe("峰值");
  });
});
