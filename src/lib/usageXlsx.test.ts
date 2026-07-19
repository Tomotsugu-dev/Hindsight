import { describe, expect, it } from "vitest";
import { buildUsageWorkbook, type XlsxCell } from "./usageXlsx";
import type { TFunction } from "i18next";
import type { UsageExportData } from "./usageExport";

const t = ((key: string) => key) as unknown as TFunction;
const labels = { t, locale: "zh-CN", fmtDuration: (min: number) => `${min}m` };

/** 两天(其中一天是"今天")+ 一周;分类:编程 > 浏览;应用:VS Code / Chrome。 */
function fixture(): UsageExportData {
  const cats = (code: number, browse: number) => [
    { id: "code", name: "编程", secs: code, minutes: Math.round(code / 60) },
    { id: "browse", name: "浏览", secs: browse, minutes: Math.round(browse / 60) },
  ];
  return {
    exportedAt: "2026-07-19T03:00:00.000Z",
    rangeStart: "2026-07-18",
    rangeEnd: "2026-07-19",
    superCategories: [{ id: "work", name: "工作", color: "#111", icon: "X", sortOrder: 0 }],
    categories: [
      // name 故意不等于内置默认名 → displayCategoryName 直接用 name,绕开 i18n
      {
        id: "code",
        name: "Coding",
        color: "#111",
        icon: "C",
        builtin: false,
        apps: [],
        superCategoryId: "work",
      },
      {
        id: "browse",
        name: "Surfing",
        color: "#222",
        icon: "B",
        builtin: false,
        apps: [],
        superCategoryId: null,
      },
    ],
    daily: [
      {
        date: "2026-07-18",
        totalSecs: 3600,
        categories: cats(3000, 600),
        apps: [
          { name: "VS Code", categoryId: "code", categoryName: "编程", minutes: 50 },
          { name: "Chrome", categoryId: "browse", categoryName: "浏览", minutes: 10 },
        ],
      },
      {
        date: "2026-07-19",
        totalSecs: 1800,
        categories: cats(1200, 600),
        apps: [{ name: "Chrome", categoryId: "browse", categoryName: "浏览", minutes: 30 }],
      },
    ],
    weekly: [
      {
        start: "2026-07-13",
        end: "2026-07-19",
        totalSecs: 5400,
        dailyAvgSecs: null,
        categories: cats(4200, 1200),
        apps: [{ name: "VS Code", categoryId: "code", categoryName: "编程", minutes: 70 }],
      },
    ],
    monthly: null,
  };
}

const text = (c?: XlsxCell): string => (c && "v" in c ? String(c.v) : "");

describe("buildUsageWorkbook v2", () => {
  const spec = buildUsageWorkbook(fixture(), labels, "ALL-DEVICES", "2026-07-19", {
    start: "2026-07-18",
    end: "2026-07-19",
    deviceId: undefined,
  });
  const names = spec.sheets.map((s) => s.name);

  it("sheet 集合:概览 + 每日 + 每周 + 应用趋势(每月未勾不出);raw 段独立", () => {
    expect(names).toEqual([
      "settings.data.export.xlsx.sheetOverview",
      "settings.data.export.granularity.daily",
      "settings.data.export.granularity.weekly",
      "settings.data.export.xlsx.sheetTrend",
    ]);
    expect(spec.raw.name).toBe("settings.data.export.xlsx.sheetRaw");
    expect(spec.raw.start).toBe("2026-07-18");
    expect(spec.raw.headers).toHaveLength(7);
    expect(spec.raw.categoryNames).toContainEqual(["code", "Coding"]);
  });

  it("概览:标题走 title 字段;关键数字四件套 + 大类合计(未归入兜底)+ Top 应用", () => {
    const overview = spec.sheets[0];
    expect(overview.title).toBe("settings.data.export.xlsx.ovTitle");
    const flat = overview.rows.map((r) => r.map(text).join("|")).join("\n");
    expect(flat).toContain("settings.data.export.xlsx.ovTotal|90m");
    expect(flat).toContain("settings.data.export.xlsx.ovDailyAvg|60m");
    expect(flat).toContain("settings.data.export.xlsx.ovActiveDays|2 / 2");
    expect(flat).toContain("settings.data.export.xlsx.ovPeakDay|2026-07-18 · 60m");
    // 大类:编程→工作;浏览无大类→未归入
    expect(flat).toContain("工作");
    expect(flat).toContain("settings.data.export.xlsx.ovUngrouped");
    // Top 应用:Chrome 两天合并 40 分钟
    const chromeRow = overview.rows.find((r) => text(r[0]) === "Chrome");
    expect(chromeRow?.[2]).toEqual({ t: "dur", v: 40 });
  });

  it("每日宽表:Table 模式、时长是 dur、0 值写空", () => {
    const daily = spec.sheets[1];
    expect(daily.table).toBe(true);
    expect(daily.hideGridlines).toBe(true);
    expect(daily.rows[1][0]).toEqual({ t: "d", v: "2026-07-18" });
    expect(daily.rows[1][2]).toEqual({ t: "dur", v: 60 });
    expect(daily.rows[1][3]).toEqual({ t: "dur", v: 50 });
  });

  it("每周:日均 null → 空单元格", () => {
    const weekly = spec.sheets[2];
    expect(weekly.rows[1][3]).toEqual({ t: "e" });
  });

  it("应用趋势:应用为行、日期为列,含「其他」缺勤补空", () => {
    const trend = spec.sheets[3];
    expect(trend.rows[0].map(text)).toEqual([
      "settings.data.export.xlsx.rawApp",
      "2026-07-18",
      "2026-07-19",
    ]);
    const vsRow = trend.rows.find((r) => text(r[0]) === "VS Code");
    expect(vsRow?.[1]).toEqual({ t: "dur", v: 50 });
    expect(vsRow?.[2]).toEqual({ t: "e" }); // 7-19 没用 VS Code
  });
});
