import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TFunction } from "i18next";
import type { AppUsage, Category, DaySummaryDto } from "../api/hindsight";

// mock 掉 Tauri invoke 层：node 环境没有 __TAURI__，且单测只关心
// 「offset 换算对不对 / 聚合口径对不对 / 序列化长什么样」
vi.mock("../api/hindsight", () => ({
  api: {
    listCategories: vi.fn(),
    listSuperCategories: vi.fn(),
    getMonthDays: vi.fn(),
    getDayApps: vi.fn(),
    getWeekDays: vi.fn(),
    getWeekApps: vi.fn(),
    getMonthApps: vi.fn(),
  },
}));

import { api } from "../api/hindsight";
import {
  collectUsageData,
  fmtLocalDate,
  MARKDOWN_TOP_APPS,
  renderUsageExport,
  usageExportFilename,
  type UsageExportData,
  type UsageExportLabels,
} from "./usageExport";

// 桩 t：回显 key，断言「有没有走对 i18n key」即可，不测翻译文本
const t = ((key: string) => key) as unknown as TFunction;
const labels: UsageExportLabels = {
  t,
  locale: "zh-CN",
  fmtDuration: (min: number) => `${min}m`,
};

/** 造一个整月的空 DaySummaryDto 序列，指定天塞 segments。 */
function monthOf(
  year: number,
  month1: number,
  filled: Record<string, DaySummaryDto["segments"]>,
): DaySummaryDto[] {
  const daysInMonth = new Date(year, month1, 0).getDate();
  return Array.from({ length: daysInMonth }, (_, i) => {
    const date = `${year}-${String(month1).padStart(2, "0")}-${String(i + 1).padStart(2, "0")}`;
    return { date, segments: filled[date] ?? [] };
  });
}

const CATS: Category[] = [
  // name 故意不等于内置默认名 → displayCategoryName 直接用 name，绕开 i18n
  {
    id: "code",
    name: "Coding",
    color: "#111111",
    icon: "Code",
    builtin: true,
    apps: [],
    superCategoryId: null,
  },
  {
    id: "browse",
    name: "Surfing",
    color: "#222222",
    icon: "Globe",
    builtin: true,
    apps: [],
    superCategoryId: null,
  },
];

const CODE_APP: AppUsage = {
  process: "Code",
  categoryId: "code",
  minutes: 60,
  iconProcess: "code.exe",
};
const CHROME_APP: AppUsage = {
  process: "Chrome",
  categoryId: "browse",
  minutes: 30,
  iconProcess: "chrome.exe",
};

describe("collectUsageData", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // 固定「今天」= 2026-07-16（周四）；6/29 是两周前的周一
    vi.setSystemTime(new Date(2026, 6, 16, 12, 0, 0));

    vi.mocked(api.listCategories).mockResolvedValue(CATS);
    vi.mocked(api.listSuperCategories).mockResolvedValue([]);
    // 6 月只有 6/30 有 1h 编程；7 月只有 7/1 有 30min 浏览
    vi.mocked(api.getMonthDays).mockImplementation((mo: number) => {
      if (mo === -1)
        return Promise.resolve(
          monthOf(2026, 6, {
            "2026-06-30": [{ categoryId: "code", minutes: 60, secs: 3600 }],
          }),
        );
      if (mo === 0)
        return Promise.resolve(
          monthOf(2026, 7, {
            "2026-07-01": [{ categoryId: "browse", minutes: 30, secs: 1800 }],
          }),
        );
      throw new Error(`unexpected month offset: ${mo}`);
    });
    vi.mocked(api.getDayApps).mockImplementation((dayOffset: number) => {
      if (dayOffset === -16) return Promise.resolve([CODE_APP]); // 6/30
      if (dayOffset === -15) return Promise.resolve([CHROME_APP]); // 7/1
      throw new Error(`unexpected day offset: ${dayOffset}`);
    });
    vi.mocked(api.getWeekDays).mockImplementation((weekOffset: number) => {
      if (weekOffset === -2)
        return Promise.resolve(
          monthOf(2026, 6, {
            "2026-06-30": [{ categoryId: "code", minutes: 60, secs: 3600 }],
          })
            .slice(28) // 6/29、6/30
            .concat(
              monthOf(2026, 7, {
                "2026-07-01": [{ categoryId: "browse", minutes: 30, secs: 1800 }],
              }).slice(0, 5), // 7/1 ~ 7/5
            ),
        );
      throw new Error(`unexpected week offset: ${weekOffset}`);
    });
    vi.mocked(api.getWeekApps).mockResolvedValue([CODE_APP, CHROME_APP]);
    vi.mocked(api.getMonthApps).mockImplementation((mo: number) =>
      Promise.resolve(mo === -1 ? [CODE_APP] : [CHROME_APP]),
    );
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("范围跨两个月：daily 按范围裁剪，weekly / monthly 整自然周期", async () => {
    const data = await collectUsageData(
      { start: "2026-06-29", end: "2026-07-02", daily: true, weekly: true, monthly: true },
      labels,
    );

    // —— daily：6/29 ~ 7/2 共 4 天，空天保留（totalSecs=0）——
    expect(data.daily?.map((d) => d.date)).toEqual([
      "2026-06-29",
      "2026-06-30",
      "2026-07-01",
      "2026-07-02",
    ]);
    expect(data.daily?.[0].totalSecs).toBe(0);
    expect(data.daily?.[1].totalSecs).toBe(3600);
    expect(data.daily?.[1].categories).toEqual([
      { id: "code", name: "Coding", secs: 3600, minutes: 60 },
    ]);
    expect(data.daily?.[1].apps[0]).toMatchObject({ name: "Code", categoryName: "Coding" });
    // 空天不查应用：只有 6/30 和 7/1 两次
    expect(api.getDayApps).toHaveBeenCalledTimes(2);

    // —— weekly：只涉及 6/29 那一个自然周，整周口径 ——
    expect(data.weekly).toHaveLength(1);
    const week = data.weekly![0];
    expect(week.start).toBe("2026-06-29");
    expect(week.end).toBe("2026-07-05");
    expect(week.totalSecs).toBe(5400);
    // 整周 7 天都早于「今天」→ 日均 = 5400 / 7
    expect(week.dailyAvgSecs).toBe(Math.round(5400 / 7));
    // 分类聚合按秒降序
    expect(week.categories.map((c) => c.id)).toEqual(["code", "browse"]);

    // —— monthly：6 月 + 7 月两个整自然月 ——
    expect(data.monthly).toHaveLength(2);
    expect(data.monthly![0]).toMatchObject({
      start: "2026-06-01",
      end: "2026-06-30",
      totalSecs: 3600,
      dailyAvgSecs: 120, // 3600 / 30 个已完成天
    });
    // 7 月进行中：已完成天 = 7/1 ~ 7/15 共 15 天
    expect(data.monthly![1]).toMatchObject({
      end: "2026-07-31",
      totalSecs: 1800,
      dailyAvgSecs: 120, // 1800 / 15
    });
  });

  it("end 晚于今天时截断到今天；未勾选的粒度为 null", async () => {
    const data = await collectUsageData(
      { start: "2026-07-01", end: "2026-12-31", daily: false, weekly: false, monthly: true },
      labels,
    );
    expect(data.rangeEnd).toBe("2026-07-16");
    expect(data.daily).toBeNull();
    expect(data.weekly).toBeNull();
    expect(data.monthly).toHaveLength(1);
    expect(api.getDayApps).not.toHaveBeenCalled();
    expect(api.getWeekDays).not.toHaveBeenCalled();
  });
});

describe("renderUsageExport", () => {
  const fixture: UsageExportData = {
    exportedAt: "2026-07-16T04:00:00.000Z",
    rangeStart: "2026-06-29",
    rangeEnd: "2026-07-02",
    superCategories: [],
    categories: CATS,
    daily: [
      {
        date: "2026-06-30",
        totalSecs: 3600,
        categories: [{ id: "code", name: "Co,ding", secs: 3600, minutes: 60 }],
        // 12 个应用：验证 Markdown 只保留 Top N（xlsx / JSON 才是全量）
        apps: Array.from({ length: 12 }, (_, i) => ({
          name: `App${i + 1}`,
          categoryId: "code",
          categoryName: "Co,ding",
          minutes: 60 - i,
        })),
      },
    ],
    weekly: null,
    monthly: null,
  };

  it("JSON：可 parse 往返，字段完整", () => {
    const parsed = JSON.parse(renderUsageExport(fixture, "json", labels)) as {
      source: string;
      daily: { date: string; totalSeconds: number; apps: unknown[] }[];
      weekly: null;
    };
    expect(parsed.source).toBe("Hindsight");
    expect(parsed.daily[0].date).toBe("2026-06-30");
    expect(parsed.daily[0].totalSeconds).toBe(3600);
    expect(parsed.daily[0].apps).toHaveLength(12);
    expect(parsed.weekly).toBeNull();
  });

  it("Markdown：走 i18n key、应用表只保留 Top N", () => {
    const md = renderUsageExport(fixture, "markdown", labels);
    expect(md).toContain("# settings.data.export.file.title");
    expect(md).toContain("## settings.data.export.file.dailyHeading");
    // 分类名原样进表格（只转义管道符，逗号不动）；时长走桩 fmtDuration
    expect(md).toContain("| Co,ding | 60m |");
    expect(md).toContain(`| App${MARKDOWN_TOP_APPS} |`);
    expect(md).not.toContain(`| App${MARKDOWN_TOP_APPS + 1} |`);
  });
});

describe("usageExportFilename / fmtLocalDate", () => {
  it("文件名按范围 + 扩展名拼接", () => {
    const range = { rangeStart: "2026-06-29", rangeEnd: "2026-07-02" };
    expect(usageExportFilename(range, "xlsx")).toBe("hindsight-usage-2026-06-29_2026-07-02.xlsx");
    expect(usageExportFilename(range, "markdown")).toBe("hindsight-usage-2026-06-29_2026-07-02.md");
  });

  it("fmtLocalDate 本地时区补零", () => {
    expect(fmtLocalDate(new Date(2026, 0, 5))).toBe("2026-01-05");
  });
});
