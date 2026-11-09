import type { UsageExportData, UsageExportLabels } from "./usageExport";

/** 「导出使用数据 → Excel 表格」的工作簿规格构建器(纯函数,vitest 直测)。
 *
 *  内容定稿(v2):
 *  1. 概览——速览仪表盘:元信息 + 关键数字四件套(总计/日均/活跃天数/峰值日)
 *     + **大类**合计表(与应用内"占比"页的信息层级一致)+ Top 10 应用表;
 *  2-4. 每日/每周/每月——宽表(小分类为列,按范围总时长降序),Excel Table 样式;
 *  5. 应用趋势——Top 15 应用 × 日期矩阵(超出合并"其他",保总量守恒),
 *     框选即可生成堆叠图;
 *  6. 明细——**原始活动记录**,由后端按 `raw` 段直查直写(十几万行不过 IPC),
 *     这里只产出它的文案与范围。
 *
 *  版式约定:时长一律 dur 单元格([h]:mm 原生格式,仍是数字),0 值写空;
 *  占比用 pct;数据表 table:true(内置样式带表头色带/斑马纹/筛选);
 *  全部 sheet 关网格线;日均沿用页面口径(只按已完成天)。 */

export type XlsxCell =
  | { t: "s"; v: string }
  | { t: "b"; v: string }
  | { t: "n"; v: number }
  | { t: "d"; v: string }
  | { t: "dur"; v: number }
  | { t: "pct"; v: number }
  | { t: "muted"; v: string }
  | { t: "note"; v: string }
  | { t: "e" };

export interface XlsxSheet {
  name: string;
  title?: string;
  table?: boolean;
  hideGridlines?: boolean;
  freezeRows?: number;
  freezeCols?: number;
  headerFilter?: boolean;
  columnWidths?: number[];
  rows: XlsxCell[][];
}

export interface XlsxRawSpec {
  name: string;
  headers: string[];
  start: string;
  end: string;
  deviceId?: string;
  categoryNames: [string, string][];
  truncatedNote: string;
}

export interface XlsxWorkbookSpec {
  sheets: XlsxSheet[];
  raw: XlsxRawSpec;
}

const S = (v: string): XlsxCell => ({ t: "s", v });
const B = (v: string): XlsxCell => ({ t: "b", v });
const D = (v: string): XlsxCell => ({ t: "d", v });
const Dur = (min: number): XlsxCell => (min > 0 ? { t: "dur", v: min } : { t: "e" });
const Pct = (frac: number): XlsxCell => ({ t: "pct", v: frac });
const Muted = (v: string): XlsxCell => ({ t: "muted", v });
const Note = (v: string): XlsxCell => ({ t: "note", v });
const E: XlsxCell = { t: "e" };

/** 趋势矩阵收录的应用数;其余合并为「其他」行。 */
export const TREND_TOP_APPS = 15;
/** 概览 Top 应用表行数。 */
export const OVERVIEW_TOP_APPS = 10;

/** 列宽按内容估算:CJK 记 2 个字符宽,再加边距;有下限防挤成一条缝。 */
function widthOf(text: string, min = 8): number {
  let w = 0;
  for (const ch of text) {
    w += ch.charCodeAt(0) > 0x2e7f ? 2 : 1;
  }
  return Math.max(min, w + 2);
}

function weekdayOf(date: string, locale: string): string {
  const [y, m, d] = date.split("-").map((v) => parseInt(v, 10));
  return new Intl.DateTimeFormat(locale, { weekday: "short" }).format(new Date(y, m - 1, d));
}

interface CatCol {
  id: string;
  name: string;
  secs: number;
}

/** 最细的已选粒度(各聚合的共同数据源;粒度间时段重叠,不能跨粒度累加)。 */
function finestPeriods(data: UsageExportData) {
  return data.daily ?? data.weekly ?? data.monthly ?? [];
}

/** 分类列:最细粒度内按总秒数降序。 */
function categoryColumns(data: UsageExportData): CatCol[] {
  const totals = new Map<string, { name: string; secs: number }>();
  for (const p of finestPeriods(data)) {
    for (const c of p.categories) {
      const cur = totals.get(c.id);
      if (cur) cur.secs += c.secs;
      else totals.set(c.id, { name: c.name, secs: c.secs });
    }
  }
  return Array.from(totals, ([id, v]) => ({ id, name: v.name, secs: v.secs })).sort(
    (a, b) => b.secs - a.secs,
  );
}

/** 应用累计(最细粒度):name+分类 → 分钟。 */
function appTotals(
  data: UsageExportData,
): { name: string; categoryName: string; minutes: number }[] {
  const totals = new Map<string, { name: string; categoryName: string; minutes: number }>();
  for (const p of finestPeriods(data)) {
    for (const a of p.apps) {
      const key = `${a.name}\u0000${a.categoryName}`;
      const cur = totals.get(key);
      if (cur) cur.minutes += a.minutes;
      else totals.set(key, { name: a.name, categoryName: a.categoryName, minutes: a.minutes });
    }
  }
  return Array.from(totals.values()).sort((a, b) => b.minutes - a.minutes);
}

export function buildUsageWorkbook(
  data: UsageExportData,
  labels: UsageExportLabels,
  deviceLabel: string,
  todayStr: string,
  rawRange: { start: string; end: string; deviceId?: string },
): XlsxWorkbookSpec {
  const { t, locale, fmtDuration } = labels;
  const k = (key: string): string => t(`settings.data.export.xlsx.${key}`);
  const granName = (g: "daily" | "weekly" | "monthly"): string =>
    t(`settings.data.export.granularity.${g}`);
  const cats = categoryColumns(data);
  const catHeaders = cats.map((c) => S(c.name));
  const catWidths = cats.map((c) => widthOf(c.name));
  const minsOf = (list: { id: string; minutes: number }[]): XlsxCell[] => {
    const byId = new Map(list.map((c) => [c.id, c.minutes]));
    return cats.map((c) => Dur(byId.get(c.id) ?? 0));
  };

  const sheets: XlsxSheet[] = [];

  // ── 1. 概览:速览仪表盘 ──
  {
    const rows: XlsxCell[][] = [
      [Muted(k("ovRange")), S(`${data.rangeStart} ~ ${data.rangeEnd}`)],
      [Muted(k("ovDevice")), S(deviceLabel)],
      [Muted(k("ovExported")), S(new Date(data.exportedAt).toLocaleString(locale))],
      [
        Muted(k("ovGranularities")),
        S(
          (["daily", "weekly", "monthly"] as const)
            .filter((g) => data[g] !== null)
            .map(granName)
            .join(" / "),
        ),
      ],
    ];

    const totalSecs = finestPeriods(data).reduce((s, p) => s + p.totalSecs, 0);

    // 关键数字四件套(需要每日粒度——唯一与所选范围精确对齐的口径)
    if (data.daily) {
      const completed = data.daily.filter((d) => d.date < todayStr);
      const avg =
        completed.length > 0
          ? Math.round(completed.reduce((s, d) => s + d.totalSecs, 0) / 60 / completed.length)
          : null;
      const active = data.daily.filter((d) => d.totalSecs > 0).length;
      const peak = data.daily.reduce(
        (best, d) => (d.totalSecs > best.totalSecs ? d : best),
        data.daily[0],
      );
      rows.push(
        [],
        [Muted(k("ovTotal")), S(fmtDuration(Math.round(totalSecs / 60)))],
        [Muted(k("ovDailyAvg")), S(avg !== null ? fmtDuration(avg) : "—")],
        [Muted(k("ovActiveDays")), S(`${active} / ${data.daily.length}`)],
        [
          Muted(k("ovPeakDay")),
          peak && peak.totalSecs > 0
            ? S(`${peak.date} · ${fmtDuration(Math.round(peak.totalSecs / 60))}`)
            : S("—"),
        ],
      );
    }

    // 大类合计表(与应用内"占比"页同层级;小分类细分在后面的宽表里)
    if (totalSecs > 0) {
      const superName = new Map(data.superCategories.map((sc) => [sc.id, sc.name]));
      const catSuper = new Map(data.categories.map((c) => [c.id, c.superCategoryId]));
      const buckets = new Map<string, number>();
      for (const c of cats) {
        const sid = catSuper.get(c.id) ?? null;
        const label = (sid !== null ? superName.get(sid) : undefined) ?? k("ovUngrouped");
        buckets.set(label, (buckets.get(label) ?? 0) + c.secs);
      }
      rows.push([], [B(k("colSuperCategory")), B(k("colDuration")), B(k("colShare"))]);
      for (const [label, secs] of Array.from(buckets.entries()).sort((a, b) => b[1] - a[1])) {
        rows.push([S(label), Dur(Math.round(secs / 60)), Pct(secs / totalSecs)]);
      }

      // Top 10 应用表——多数人要的最终答案,不藏在最后
      const top = appTotals(data);
      if (top.length > 0) {
        const totalMin = totalSecs / 60;
        rows.push([], [B(k("rawApp")), B(k("rawCategory")), B(k("colDuration")), B(k("colShare"))]);
        for (const a of top.slice(0, OVERVIEW_TOP_APPS)) {
          rows.push([
            S(a.name),
            S(a.categoryName),
            Dur(a.minutes),
            Pct(totalMin > 0 ? a.minutes / totalMin : 0),
          ]);
        }
      }
    } else {
      rows.push([], [S(k("noData"))]);
    }
    rows.push([], [Note(k("note"))]);
    sheets.push({
      name: k("sheetOverview"),
      title: k("ovTitle"),
      hideGridlines: true,
      columnWidths: [18, 22, 10, 10],
      rows,
    });
  }

  // ── 2. 每日(宽表 Table) ──
  if (data.daily) {
    const header = [S(k("colDate")), S(k("colWeekday")), S(k("colTotal")), ...catHeaders];
    const rows: XlsxCell[][] = [header];
    for (const d of data.daily) {
      rows.push([
        D(d.date),
        S(weekdayOf(d.date, locale)),
        Dur(Math.round(d.totalSecs / 60)),
        ...minsOf(d.categories),
      ]);
    }
    sheets.push({
      name: granName("daily"),
      table: true,
      hideGridlines: true,
      freezeRows: 1,
      freezeCols: 1,
      columnWidths: [11, widthOf(k("colWeekday"), 7), widthOf(k("colTotal"), 8), ...catWidths],
      rows,
    });
  }

  // ── 3/4. 每周 / 每月(宽表 Table) ──
  const periodSheet = (
    granularity: "weekly" | "monthly",
    periods: NonNullable<UsageExportData["weekly"]>,
  ): XlsxSheet => {
    const monthly = granularity === "monthly";
    const header = [
      ...(monthly ? [S(k("colMonth"))] : []),
      S(k("colStart")),
      S(k("colEnd")),
      S(k("colTotal")),
      S(k("colDailyAvg")),
      ...catHeaders,
    ];
    const rows: XlsxCell[][] = [header];
    for (const p of periods) {
      rows.push([
        ...(monthly ? [S(p.start.slice(0, 7))] : []),
        D(p.start),
        D(p.end),
        Dur(Math.round(p.totalSecs / 60)),
        p.dailyAvgSecs !== null ? Dur(Math.round(p.dailyAvgSecs / 60)) : E,
        ...minsOf(p.categories),
      ]);
    }
    return {
      name: granName(granularity),
      table: true,
      hideGridlines: true,
      freezeRows: 1,
      freezeCols: monthly ? 1 : 0,
      columnWidths: [
        ...(monthly ? [9] : []),
        11,
        11,
        widthOf(k("colTotal"), 8),
        widthOf(k("colDailyAvg"), 8),
        ...catWidths,
      ],
      rows,
    };
  };
  if (data.weekly) sheets.push(periodSheet("weekly", data.weekly));
  if (data.monthly) sheets.push(periodSheet("monthly", data.monthly));

  // ── 5. 应用趋势:Top N 应用 × 日期矩阵(需每日粒度) ──
  if (data.daily && data.daily.length > 0) {
    const top = appTotals(data)
      .slice(0, TREND_TOP_APPS)
      .map((a) => a.name);
    const topSet = new Set(top);
    const dates = data.daily.map((d) => d.date);
    // 每应用×每日分钟;非 Top 归入「其他」
    const perApp = new Map<string, Map<string, number>>();
    const other = new Map<string, number>();
    for (const d of data.daily) {
      for (const a of d.apps) {
        if (topSet.has(a.name)) {
          const m = perApp.get(a.name) ?? new Map<string, number>();
          m.set(d.date, (m.get(d.date) ?? 0) + a.minutes);
          perApp.set(a.name, m);
        } else {
          other.set(d.date, (other.get(d.date) ?? 0) + a.minutes);
        }
      }
    }
    const rows: XlsxCell[][] = [[S(k("rawApp")), ...dates.map((d) => S(d))]];
    for (const name of top) {
      const m = perApp.get(name) ?? new Map<string, number>();
      rows.push([S(name), ...dates.map((d) => Dur(m.get(d) ?? 0))]);
    }
    if (other.size > 0) {
      rows.push([S(k("trendOther")), ...dates.map((d) => Dur(other.get(d) ?? 0))]);
    }
    sheets.push({
      name: k("sheetTrend"),
      table: true,
      hideGridlines: true,
      freezeRows: 1,
      freezeCols: 1,
      columnWidths: [
        Math.min(Math.max(...top.map((n) => widthOf(n, 12)), 12), 30),
        ...dates.map(() => 11),
      ],
      rows,
    });
  }

  // ── 6. 明细(原始活动记录):后端按此段直查直写 ──
  const raw: XlsxRawSpec = {
    name: k("sheetRaw"),
    headers: [
      k("rawDate"),
      k("rawStart"),
      k("colDuration"),
      k("rawApp"),
      k("rawTitle"),
      k("rawCategory"),
      k("rawDevice"),
    ],
    start: rawRange.start,
    end: rawRange.end,
    deviceId: rawRange.deviceId,
    categoryNames: data.categories.map((c) => [c.id, c.name]),
    truncatedNote: k("rawTruncated"),
  };

  return { sheets, raw };
}
