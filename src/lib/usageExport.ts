import type { TFunction } from "i18next";
import {
  api,
  type AppUsage,
  type Category,
  type DaySummaryDto,
  type SuperCategory,
} from "../api/hindsight";
import { displayCategoryName } from "../utils/categoryName";

/** 「设置 → 数据 → 导出使用数据」的核心逻辑：按日期范围拉统计 + 序列化成文件文本。
 *
 *  设计约束（对应导出弹窗的选项）：
 *  - 只导**统计数据**（周期总时长 / 分类时长 / 应用时长），不导原始活动记录——
 *    后端也没有暴露原始 activities 的查询命令
 *  - 周 / 月统计按**完整自然周（周一～周日）/ 自然月**计算，跟周 / 月页面显示的数字
 *    一致；周期与所选范围部分重叠时也整周期导出（条目上标注了周期起止，不会误读）
 *  - 日均口径同周 / 月页面：只按「严格早于今天的已完成天」算（见 completedDaysOf）
 *  - 全部查询走现有报表命令（get_month_days / get_week_apps / ...），零后端改动 */

export type UsageExportFormat = "json" | "markdown" | "xlsx";

/** 文本格式(renderUsageExport 的定义域);xlsx 走 lib/usageXlsx.ts + 后端写入器。 */
export type UsageTextFormat = Exclude<UsageExportFormat, "xlsx">;

export interface UsageExportOptions {
  /** 范围起（含），"YYYY-MM-DD" */
  start: string;
  /** 范围止（含），"YYYY-MM-DD"；晚于今天时按今天截断 */
  end: string;
  daily: boolean;
  weekly: boolean;
  monthly: boolean;
}

/** 序列化需要的本地化上下文。collect 阶段就要（分类名固化成当前语言文本）。 */
export interface UsageExportLabels {
  t: TFunction;
  /** i18n.language，喂给 Intl 做星期几 */
  locale: string;
  /** useDurationFormatter 的结果（"X 小时 Y 分"），Markdown 用 */
  fmtDuration: (min: number) => string;
}

/** 单个周期内一个分类的用时。name 是导出时语言下的显示名（builtin 走 i18n）。 */
interface CatStat {
  id: string;
  name: string;
  secs: number;
  minutes: number;
}

/** 单个周期内一个应用（组）的用时。后端只给整数分钟，没有精确秒。 */
interface AppStat {
  name: string;
  categoryId: string;
  categoryName: string;
  minutes: number;
}

interface DayStat {
  /** "YYYY-MM-DD" */
  date: string;
  totalSecs: number;
  categories: CatStat[];
  apps: AppStat[];
}

interface PeriodStat {
  /** 周期起止（含），"YYYY-MM-DD"；周=周一～周日，月=1 号～月末 */
  start: string;
  end: string;
  totalSecs: number;
  /** 日均秒数（已完成天口径）；周期内没有已完成天时为 null */
  dailyAvgSecs: number | null;
  categories: CatStat[];
  apps: AppStat[];
}

/** collect 出来的中间数据；三种格式都从这一份渲染。 */
export interface UsageExportData {
  /** RFC3339 UTC */
  exportedAt: string;
  rangeStart: string;
  rangeEnd: string;
  superCategories: SuperCategory[];
  categories: Category[];
  daily: DayStat[] | null;
  weekly: PeriodStat[] | null;
  monthly: PeriodStat[] | null;
}

/** Markdown 是给人读的：应用表只保留 Top N（xlsx / JSON 全量）。 */
export const MARKDOWN_TOP_APPS = 10;

/** 「全量应用」的 limit——后端 SQL `LIMIT ?` 必须给个数，取一个远超单周期
 *  可能应用数的值。 */
const ALL_APPS_LIMIT = 100_000;

/** 并发拉数的批大小：本地 SQLite 查询很快，限一下避免一次塞几百个 invoke。 */
const FETCH_CHUNK = 8;

const DAY_MS = 86_400_000;

// ---------------------------------------------------------------------------
// 日期助手（全部本地时区；周期换算跟后端 reports.rs 的 Local::now() 口径一致）
// ---------------------------------------------------------------------------

function startOfDay(d: Date): Date {
  const out = new Date(d);
  out.setHours(0, 0, 0, 0);
  return out;
}

/** Date → "YYYY-MM-DD"（本地时区；toISOString 是 UTC，东侧时区会偏一天）。 */
export function fmtLocalDate(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function addDays(d: Date, n: number): Date {
  const out = new Date(d);
  out.setDate(out.getDate() + n);
  return out;
}

/** 两个本地零点日期的整天差。除法后 round 抹掉 DST 造成的 ±1h 偏差。 */
function diffDays(a: Date, b: Date): number {
  return Math.round((a.getTime() - b.getTime()) / DAY_MS);
}

/** 所在周的周一（后端周口径 = 周一～周日）。getDay(): 0=周日。 */
function mondayOf(d: Date): Date {
  const day = d.getDay();
  return addDays(d, day === 0 ? -6 : 1 - day);
}

/** 把 items 分批并发跑 fn，保持结果顺序。 */
async function mapChunked<T, R>(
  items: T[],
  fn: (item: T, index: number) => Promise<R>,
  chunkSize = FETCH_CHUNK,
): Promise<R[]> {
  const out: R[] = [];
  for (let i = 0; i < items.length; i += chunkSize) {
    const batch = items.slice(i, i + chunkSize);
    out.push(...(await Promise.all(batch.map((item, j) => fn(item, i + j)))));
  }
  return out;
}

// ---------------------------------------------------------------------------
// 数据收集
// ---------------------------------------------------------------------------

/** 把一批天的 segments 按分类累秒（先加总后取整，跟 catMinutesFromSegments 同口径）。 */
function catStatsFromDays(days: DaySummaryDto[], catName: (id: string) => string): CatStat[] {
  const totals = new Map<string, number>();
  for (const d of days) {
    for (const seg of d.segments) {
      totals.set(seg.categoryId, (totals.get(seg.categoryId) ?? 0) + seg.secs);
    }
  }
  return Array.from(totals, ([id, secs]) => ({
    id,
    name: catName(id),
    secs,
    minutes: Math.round(secs / 60),
  })).sort((a, b) => b.secs - a.secs);
}

function totalSecsOfDays(days: DaySummaryDto[]): number {
  return days.reduce((sum, d) => sum + d.segments.reduce((s, seg) => s + seg.secs, 0), 0);
}

function appStats(apps: AppUsage[], catName: (id: string) => string): AppStat[] {
  return apps.map((a) => ({
    name: a.process,
    categoryId: a.categoryId,
    categoryName: catName(a.categoryId),
    minutes: a.minutes,
  }));
}

/** 周期的日均（秒）：只按「严格早于今天的已完成天」算，分子分母一起排除今天，
 *  跟周 / 月页面的 completedDaysOf 口径一致。 */
function dailyAvgSecs(days: DaySummaryDto[], todayStr: string): number | null {
  const completed = days.filter((d) => d.date < todayStr);
  if (completed.length === 0) return null;
  return Math.round(totalSecsOfDays(completed) / completed.length);
}

/** 按选项拉全部统计数据。调用量 ≈ 范围天数（每日应用）+ 每周 2 次 + 每月 2 次，
 *  本地 SQLite 单次毫秒级，90 天范围约一两秒。 */
export async function collectUsageData(
  opts: UsageExportOptions,
  labels: UsageExportLabels,
): Promise<UsageExportData> {
  const today = startOfDay(new Date());
  const todayStr = fmtLocalDate(today);
  const start = new Date(...splitDate(opts.start));
  // end 不晚于今天：未来的天没有数据，offset 也不应为正
  const endParsed = new Date(...splitDate(opts.end));
  const end = endParsed > today ? today : endParsed;
  const startStr = fmtLocalDate(start);
  const endStr = fmtLocalDate(end);

  const [categories, superCategories] = await Promise.all([
    api.listCategories(),
    api.listSuperCategories(),
  ]);
  const catById = new Map(categories.map((c) => [c.id, c]));
  const catName = (id: string): string => {
    const cat = catById.get(id);
    return cat ? displayCategoryName(cat, labels.t) : id;
  };

  // —— 涉及的自然月：daily 与 monthly 共用同一批 get_month_days 结果 ——
  const needMonths = opts.daily || opts.monthly;
  const monthOffsets: number[] = [];
  if (needMonths) {
    const first = new Date(start.getFullYear(), start.getMonth(), 1);
    const base = today.getFullYear() * 12 + today.getMonth();
    for (let cur = first; cur <= end; cur = new Date(cur.getFullYear(), cur.getMonth() + 1, 1)) {
      monthOffsets.push(cur.getFullYear() * 12 + cur.getMonth() - base);
    }
  }
  const monthDays = await mapChunked(monthOffsets, (mo) => api.getMonthDays(mo));

  // —— 每日：月数据 clip 到所选范围；有活动的天再补一次全量应用 ——
  let daily: DayStat[] | null = null;
  if (opts.daily) {
    const dayDtos = monthDays.flat().filter((d) => d.date >= startStr && d.date <= endStr);
    daily = await mapChunked(dayDtos, async (d) => {
      const totalSecs = totalSecsOfDays([d]);
      const apps =
        totalSecs > 0
          ? await api.getDayApps(diffDays(new Date(...splitDate(d.date)), today), ALL_APPS_LIMIT)
          : [];
      return {
        date: d.date,
        totalSecs,
        categories: catStatsFromDays([d], catName),
        apps: appStats(apps, catName),
      };
    });
  }

  // —— 每周：范围涉及的每个自然周，整周口径 ——
  let weekly: PeriodStat[] | null = null;
  if (opts.weekly) {
    const thisMonday = mondayOf(today);
    const mondays: Date[] = [];
    for (let cur = mondayOf(start); cur <= end; cur = addDays(cur, 7)) {
      mondays.push(cur);
    }
    weekly = await mapChunked(mondays, async (monday) => {
      const weekOffset = Math.round(diffDays(monday, thisMonday) / 7);
      const [days, apps] = await Promise.all([
        api.getWeekDays(weekOffset),
        api.getWeekApps(weekOffset, ALL_APPS_LIMIT),
      ]);
      return {
        start: fmtLocalDate(monday),
        end: fmtLocalDate(addDays(monday, 6)),
        totalSecs: totalSecsOfDays(days),
        dailyAvgSecs: dailyAvgSecs(days, todayStr),
        categories: catStatsFromDays(days, catName),
        apps: appStats(apps, catName),
      };
    });
  }

  // —— 每月：get_month_days 结果已在手，补每月全量应用 ——
  let monthly: PeriodStat[] | null = null;
  if (opts.monthly) {
    monthly = await mapChunked(monthOffsets, async (mo, i) => {
      const days = monthDays[i];
      const apps = await api.getMonthApps(mo, ALL_APPS_LIMIT);
      return {
        start: days[0].date,
        end: days[days.length - 1].date,
        totalSecs: totalSecsOfDays(days),
        dailyAvgSecs: dailyAvgSecs(days, todayStr),
        categories: catStatsFromDays(days, catName),
        apps: appStats(apps, catName),
      };
    });
  }

  return {
    exportedAt: new Date().toISOString(),
    rangeStart: startStr,
    rangeEnd: endStr,
    superCategories,
    categories,
    daily,
    weekly,
    monthly,
  };
}

/** "YYYY-MM-DD" → new Date(y, m-1, d) 的参数三元组（本地时区解析，避免
 *  new Date("YYYY-MM-DD") 的 UTC 解析在东侧时区偏移一天）。 */
function splitDate(s: string): [number, number, number] {
  const [y, m, d] = s.split("-").map((v) => parseInt(v, 10));
  return [y, m - 1, d];
}

// ---------------------------------------------------------------------------
// 序列化
// ---------------------------------------------------------------------------

export function renderUsageExport(
  data: UsageExportData,
  format: UsageTextFormat,
  labels: UsageExportLabels,
): string {
  switch (format) {
    case "json":
      return renderJson(data);
    case "markdown":
      return renderMarkdown(data, labels);
  }
}

export function usageExportFilename(
  data: Pick<UsageExportData, "rangeStart" | "rangeEnd">,
  format: UsageExportFormat,
): string {
  const ext = format === "markdown" ? "md" : format;
  return `hindsight-usage-${data.rangeStart}_${data.rangeEnd}.${ext}`;
}

// —— JSON：字段最全（分类 ID / 大类 / 精确秒 / 日均），给程序处理。 ——

function renderJson(data: UsageExportData): string {
  const catsJson = (cats: CatStat[]): Record<string, unknown>[] =>
    cats.map((c) => ({ id: c.id, name: c.name, seconds: c.secs, minutes: c.minutes }));
  const appsJson = (apps: AppStat[]): Record<string, unknown>[] =>
    apps.map((a) => ({ name: a.name, categoryId: a.categoryId, minutes: a.minutes }));
  const period = (p: PeriodStat): Record<string, unknown> => ({
    start: p.start,
    end: p.end,
    totalSeconds: p.totalSecs,
    dailyAverageSeconds: p.dailyAvgSecs,
    categories: catsJson(p.categories),
    apps: appsJson(p.apps),
  });

  return JSON.stringify(
    {
      source: "Hindsight",
      type: "usage-statistics",
      version: 1,
      exportedAt: data.exportedAt,
      range: { start: data.rangeStart, end: data.rangeEnd },
      superCategories: data.superCategories.map((s) => ({
        id: s.id,
        name: s.name,
        color: s.color,
      })),
      categories: data.categories.map((c) => ({
        id: c.id,
        name: c.name,
        color: c.color,
        superCategoryId: c.superCategoryId,
      })),
      daily:
        data.daily?.map((d) => ({
          date: d.date,
          totalSeconds: d.totalSecs,
          categories: catsJson(d.categories),
          apps: appsJson(d.apps),
        })) ?? null,
      weekly: data.weekly?.map(period) ?? null,
      monthly: data.monthly?.map(period) ?? null,
    },
    null,
    2,
  );
}

// —— Markdown：给人读的报告。应用表只留 Top N，表格转义管道符。 ——

function mdEscape(s: string): string {
  return s.replace(/\|/g, "\\|");
}

function renderMarkdown(data: UsageExportData, labels: UsageExportLabels): string {
  const { t, locale, fmtDuration } = labels;
  const weekdayFmt = new Intl.DateTimeFormat(locale, { weekday: "short" });
  const dur = (secs: number): string => fmtDuration(Math.round(secs / 60));
  const lines: string[] = [];

  lines.push(
    `# ${t("settings.data.export.file.title", { start: data.rangeStart, end: data.rangeEnd })}`,
    "",
    `- ${t("settings.data.export.file.metaExportedAt", { time: new Date(data.exportedAt).toLocaleString(locale) })}`,
    `- ${t("settings.data.export.file.metaRange", { start: data.rangeStart, end: data.rangeEnd })}`,
    `- ${t("settings.data.export.file.metaNote", { n: MARKDOWN_TOP_APPS })}`,
    "",
  );

  const pushTables = (stat: {
    totalSecs: number;
    categories: CatStat[];
    apps: AppStat[];
  }): void => {
    if (stat.totalSecs <= 0) {
      lines.push(t("settings.data.export.file.noData"), "");
      return;
    }
    lines.push(
      `| ${t("settings.data.export.file.colCategory")} | ${t("settings.data.export.file.colDuration")} | ${t("settings.data.export.file.colShare")} |`,
      "| --- | ---: | ---: |",
    );
    for (const c of stat.categories) {
      const share = Math.round((c.secs / stat.totalSecs) * 100);
      lines.push(`| ${mdEscape(c.name)} | ${dur(c.secs)} | ${share}% |`);
    }
    lines.push("");
    if (stat.apps.length > 0) {
      lines.push(
        `**${t("settings.data.export.file.topAppsTitle", { n: MARKDOWN_TOP_APPS })}**`,
        "",
        `| # | ${t("settings.data.export.file.colApp")} | ${t("settings.data.export.file.colCategory")} | ${t("settings.data.export.file.colDuration")} |`,
        "| ---: | --- | --- | ---: |",
      );
      stat.apps.slice(0, MARKDOWN_TOP_APPS).forEach((a, i) => {
        lines.push(
          `| ${i + 1} | ${mdEscape(a.name)} | ${mdEscape(a.categoryName)} | ${fmtDuration(a.minutes)} |`,
        );
      });
      lines.push("");
    }
  };

  const totalLabel = (secs: number): string =>
    `${t("settings.data.export.file.total")} ${dur(secs)}`;
  const avgLabel = (avg: number | null): string =>
    avg === null ? "" : ` · ${t("settings.data.export.file.dailyAvg")} ${dur(avg)}`;

  // 周期标题的括号 / 范围分隔符按语言走 i18n 模板（中文全角、西文半角）
  if (data.daily) {
    lines.push(`## ${t("settings.data.export.file.dailyHeading")}`, "");
    for (const d of data.daily) {
      const heading = t("settings.data.export.file.dayHeading", {
        date: d.date,
        weekday: weekdayFmt.format(new Date(...splitDate(d.date))),
      });
      lines.push(`### ${heading} · ${totalLabel(d.totalSecs)}`, "");
      pushTables(d);
    }
  }
  if (data.weekly) {
    lines.push(`## ${t("settings.data.export.file.weeklyHeading")}`, "");
    for (const w of data.weekly) {
      const heading = t("settings.data.export.file.weekHeading", {
        start: w.start,
        end: w.end,
      });
      lines.push(`### ${heading} · ${totalLabel(w.totalSecs)}${avgLabel(w.dailyAvgSecs)}`, "");
      pushTables(w);
    }
  }
  if (data.monthly) {
    lines.push(`## ${t("settings.data.export.file.monthlyHeading")}`, "");
    for (const m of data.monthly) {
      const heading = t("settings.data.export.file.monthHeading", {
        month: m.start.slice(0, 7),
        start: m.start,
        end: m.end,
      });
      lines.push(`### ${heading} · ${totalLabel(m.totalSecs)}${avgLabel(m.dailyAvgSecs)}`, "");
      pushTables(m);
    }
  }

  return lines.join("\n");
}
