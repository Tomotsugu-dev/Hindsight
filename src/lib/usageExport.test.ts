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
    getAppIconDataUrl: vi.fn(),
  },
}));

import { api } from "../api/hindsight";
import {
  collectAppIcons,
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
          iconProcess: `app${i + 1}.exe`,
        })),
      },
    ],
    weekly: null,
    monthly: null,
  };

  // —— HTML：自包含单文件报告 ——

  it("HTML：单文件自包含,不引用任何外部资源", () => {
    const html = renderUsageExport(fixture, "html", labels);
    expect(html.startsWith("<!doctype html>")).toBe(true);
    expect(html).toContain("<style>");
    // 离线可读是这个格式存在的理由:一旦引外链,断网/转发后就散架。
    // 交互脚本允许存在,但必须内联——不许 <script src>。
    expect(html).toContain("<script>");
    expect(html).not.toMatch(/<script[^>]*\ssrc=/i);
    expect(html).not.toMatch(/src=["']https?:/i);
    expect(html).not.toMatch(/<link[^>]+stylesheet/i);
  });

  it("HTML：图表用分类自己的颜色,趋势柱与占比条为内联元素", () => {
    const html = renderUsageExport(fixture, "html", labels);
    expect(html).toContain("sharebar");
    expect(html).toContain("trend-inner");
    // CATS 里 code 分类的颜色必须出现在图表/色块里
    const codeColor = CATS.find((c) => c.id === "code")!.color;
    expect(html).toContain(codeColor);
  });

  it("HTML：应用行嵌入图标 data URL,拿不到图标时用分类色 fallback", () => {
    const icons = new Map([["app1.exe", "data:image/png;base64,AAAA"]]);
    const html = renderUsageExport(fixture, "html", labels, icons);
    // 图标走 <style> 里的类,单元格只留一个短类名引用
    expect(html).toContain('background-image:url("data:image/png;base64,AAAA")');
    expect(html).toMatch(/class="app-icon ic\d+"/);
    // 其余 11 个应用没有 data URL → 走分类色首字符 fallback
    expect(html).toContain("app-fallback");
  });

  it("HTML：柱高按总时长精确算,段高是柱内占比", () => {
    const html = renderUsageExport(fixture, "html", labels);
    // 整柱高度内联在 stack 上(唯一一根柱 = max → 正好 120px)
    expect(html).toMatch(/class="stack" style="height:120(\.0)?px"/);
    // 段不再各给 2px 下限:高度是百分比,细分类多也垫不高柱子
    expect(html).toMatch(/<i style="height:\d+(\.\d+)?%;background:/);
    // 日期标签必须定高:空标签塌陷成 0 会让那根柱下沉,整排柱底出锯齿
    expect(html).toMatch(/\.tlabel\{[^}]*height:15px\}/);
  });

  it("HTML：悬停明细走自绘浮层,图表元素不挂原生 title", () => {
    const html = renderUsageExport(fixture, "html", labels);
    // 柱子 / 占比条段 / 跳转胶囊都带 data-tip,由页尾脚本画浮层
    expect(html).toContain('data-tip="');
    expect(html).toContain('document.addEventListener("mouseover"');
    // 原生 title 裸框退场
    expect(html).not.toMatch(/<i style="[^"]*" title=/);
    expect(html).not.toMatch(/<a class="col"[^>]*title=/);
  });

  it("HTML：柱顶数值抽稀时,最高的柱必须有数", () => {
    // 40 天,峰值放在 idx=7(不是步长倍数):抽稀不能把最高柱的数字抽掉
    const day = fixture.daily![0];
    const many: UsageExportData = {
      ...fixture,
      daily: Array.from({ length: 40 }, (_, i) => ({
        ...day,
        date: `2026-07-${String(i + 1).padStart(2, "0")}`,
        totalSecs: i === 7 ? 100_000 : 3_600,
      })),
    };
    const html = renderUsageExport(many, "html", labels);
    // 100000s = 27.8h,只有峰值柱是这个值
    expect(html).toContain(">27.8h</span>");
  });

  it("HTML：同一图标的 base64 全文只出现一次", () => {
    // 一个应用在 30 天 × 日/周/月里要出现几十上百回。曾经每处一份 base64,
    // 把一份月报顶到 32 MB —— 这条锁住「只嵌一次」。
    const day = fixture.daily![0];
    const many: UsageExportData = {
      ...fixture,
      daily: Array.from({ length: 30 }, (_, i) => ({
        ...day,
        date: `2026-06-${String(i + 1).padStart(2, "0")}`,
      })),
      weekly: Array.from({ length: 4 }, (_, i) => ({
        start: `2026-06-0${i + 1}`,
        end: `2026-06-0${i + 7}`,
        totalSecs: day.totalSecs,
        dailyAvgSecs: 3600,
        categories: day.categories,
        apps: day.apps,
      })),
      monthly: [
        {
          start: "2026-06-01",
          end: "2026-06-30",
          totalSecs: day.totalSecs,
          dailyAvgSecs: 3600,
          categories: day.categories,
          apps: day.apps,
        },
      ],
    };
    const icons = new Map([["app1.exe", "data:image/png;base64,AAAA"]]);
    const html = renderUsageExport(many, "html", labels, icons);
    expect(html.split("base64,AAAA").length - 1).toBe(1);
    // 35 个周期各引用一次类名
    expect(html.split('class="app-icon ic0"').length - 1).toBe(35);
  });

  it("HTML：每个周期都有锚点,顶部与章节里都能直接跳过去", () => {
    const html = renderUsageExport(fixture, "html", labels);
    // 吸顶条 → 章节
    expect(html).toContain('href="#daily"');
    expect(html).toContain('<section class="section" id="daily">');
    // 章节里的跳转胶囊 + 趋势柱 → 具体某天的卡片
    expect(html).toContain('class="jump"');
    expect(html).toContain('href="#d-2026-06-30"');
    expect(html).toContain('<div class="pd" id="d-2026-06-30">');
    // 跳到折叠行时脚本把它自动点开,落地不是一条收着的行
    expect(html).toContain('addEventListener("hashchange"');
  });

  it("HTML：一个周期收成一行,默认折叠,展开不靠 <details>", () => {
    const html = renderUsageExport(fixture, "html", labels);
    expect(html).toContain('<label class="row"');
    expect(html).toContain('<input class="tgl" type="checkbox"');
    // <details> 的折叠内容藏在浏览器 shadow DOM 里,Ctrl+P 摊不平(实测只出汇总行)
    expect(html).not.toMatch(/<details/i);
    // 打印时把所有展开区摊开,纸上不该丢明细
    expect(html).toMatch(/@media print\{[\s\S]*\.det\{display:block\}/);

    // 只有一天时默认摊开;多天时全部收起,一屏扫得完
    expect(html).toContain('type="checkbox" id="t-d-2026-06-30" checked>');
    const many: UsageExportData = {
      ...fixture,
      daily: Array.from({ length: 5 }, (_, i) => ({
        ...fixture.daily![0],
        date: `2026-06-0${i + 1}`,
      })),
    };
    expect(renderUsageExport(many, "html", labels)).not.toContain('checkbox" id="t-d-2026-06-01" checked');
  });

  it("HTML：用户数据全部转义,不会被当成标签解析", () => {
    const evil: UsageExportData = {
      ...fixture,
      daily: [
        {
          date: "2026-06-30",
          totalSecs: 60,
          categories: [{ id: "code", name: "<img src=x onerror=alert(1)>", secs: 60, minutes: 1 }],
          apps: [
            {
              name: '"><script>alert(1)</script>',
              categoryId: "code",
              categoryName: "a & b",
              minutes: 1,
            },
          ],
        },
      ],
    };
    const html = renderUsageExport(evil, "html", labels);
    expect(html).not.toContain("<img src=x");
    expect(html).not.toContain("<script>alert");
    expect(html).toContain("&lt;img src=x");
    expect(html).toContain("a &amp; b");
  });

  it("HTML：无数据的周期显示占位而不是空白卡片", () => {
    const empty: UsageExportData = {
      ...fixture,
      daily: [{ date: "2026-06-30", totalSecs: 0, categories: [], apps: [] }],
    };
    const html = renderUsageExport(empty, "html", labels);
    expect(html).toContain("noData");
  });

  it("HTML：文件名用 .html 扩展名", () => {
    expect(usageExportFilename(fixture, "html")).toBe(
      "hindsight-usage-2026-06-29_2026-07-02.html",
    );
  });

  it("collectAppIcons：只收集会渲染的 Top N 唯一进程,拿不到图标的只跳过", async () => {
    vi.mocked(api.getAppIconDataUrl).mockImplementation((p: string) =>
      Promise.resolve(p === "app1.exe" ? "data:image/png;base64,x" : null),
    );
    const icons = await collectAppIcons(fixture);
    expect(icons.get("app1.exe")).toBe("data:image/png;base64,x");
    // 12 个应用只有 Top 10 会渲染;其余返回 null 不进 map
    expect([...icons.keys()]).toEqual(["app1.exe"]);
    expect(vi.mocked(api.getAppIconDataUrl).mock.calls).toHaveLength(10);
  });

  it("collectAppIcons：无数据的周期不触发图标查询", async () => {
    vi.mocked(api.getAppIconDataUrl).mockClear();
    const empty: UsageExportData = {
      ...fixture,
      daily: [{ date: "2026-06-30", totalSecs: 0, categories: [], apps: [] }],
    };
    const icons = await collectAppIcons(empty);
    expect(icons.size).toBe(0);
    expect(api.getAppIconDataUrl).not.toHaveBeenCalled();
  });

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
