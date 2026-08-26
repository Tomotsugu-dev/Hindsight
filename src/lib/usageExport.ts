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

export type UsageExportFormat = "json" | "markdown" | "html" | "xlsx";

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
  /** 应用图标的代表进程名（HTML 报告内嵌图标用；Markdown / JSON 用不到）。 */
  iconProcess?: string;
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
    iconProcess: a.iconProcess,
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

/** HTML 报告里会渲染进应用表的应用集合（Top N 且有数据）的唯一进程名。 */
export function htmlIconProcesses(data: UsageExportData): string[] {
  const wanted = new Set<string>();
  const push = (list: AppStat[] | undefined): void => {
    for (const a of (list ?? []).slice(0, MARKDOWN_TOP_APPS)) {
      if (a.iconProcess) wanted.add(a.iconProcess);
    }
  };
  // 与 periodCardHtml 口径一致：只有周期有数据时应用表才会渲染
  for (const p of [data.daily ?? [], data.weekly ?? [], data.monthly ?? []]) {
    for (const it of p) if (it.totalSecs > 0) push(it.apps);
  }
  return [...wanted];
}

/** 给 HTML 报告收集应用图标：唯一进程名 → base64 data URL。
 *  单个图标失败只跳过不报错——报告本身照常导出。 */
export async function collectAppIcons(data: UsageExportData): Promise<Map<string, string>> {
  const out = new Map<string, string>();
  await mapChunked(htmlIconProcesses(data), async (name) => {
    try {
      const url = await api.getAppIconDataUrl(name);
      if (url) out.set(name, url);
    } catch {
      // 图标拿不到就走报告里的分类色 fallback
    }
  });
  return out;
}

export function renderUsageExport(
  data: UsageExportData,
  format: UsageTextFormat,
  labels: UsageExportLabels,
  icons?: Map<string, string>,
): string {
  switch (format) {
    case "json":
      return renderJson(data);
    case "markdown":
      return renderMarkdown(data, labels);
    case "html":
      return renderHtml(data, labels, icons);
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

// —— HTML：自包含单文件报告，双击即开。Markdown 的表格干巴巴，Excel 又要装软件；
//    这条路把图表和配色一起嵌进去，离线可看、可分享、可打印。 ——

/** HTML 属性/正文转义。数据里带用户自定义的分类名与窗口标题，必须转义。 */
function htmlEscape(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** 分类 id → 用户配的颜色；查不到给中性灰（builtin/已删分类都可能落这里）。 */
function catColorMap(data: UsageExportData): Map<string, string> {
  const m = new Map<string, string>();
  for (const c of data.categories) m.set(c.id, c.color);
  return m;
}

/** 一行横向占比条：按分类颜色分段，宽度 = 该分类占比（CSS flex，打印友好）。 */
function shareBar(
  cats: CatStat[],
  totalSecs: number,
  colors: Map<string, string>,
  dur: (secs: number) => string,
): string {
  if (totalSecs <= 0) return `<span class="sharebar empty-bar"></span>`;
  const segs = cats
    .filter((c) => c.secs > 0)
    .map((c) => {
      const pct = (c.secs / totalSecs) * 100;
      const color = htmlEscape(colors.get(c.id) ?? "#94a3b8");
      const tip = `${c.name} · ${pct.toFixed(0)}% · ${dur(c.secs)}`;
      return (
        `<i style="width:${pct.toFixed(2)}%;background:${color}" ` +
        `data-tip="${htmlEscape(tip)}"></i>`
      );
    })
    .join("");
  return `<span class="sharebar">${segs}</span>`;
}

interface TrendItem {
  label: string;
  shortLabel: string;
  totalSecs: number;
  categories: CatStat[];
  /** 柱子/胶囊指向的折叠块 id。 */
  anchor: string;
}

/** 紧凑小时数（柱顶数值标签用）：20 小时 41 分 → "20.7h"。 */
function shortHours(secs: number): string {
  return `${(secs / 3600).toFixed(1)}h`;
}

/** 周期趋势柱状图：每根柱 = 一个周期总时长，高度按小时线性（柱顶标注小时数），
 *  颜色按分类自下而上堆叠。柱少时（周 / 月）定宽居中，别撑满整行。
 *  每根柱都是通往那个周期的链接——看见高峰想细看，点它最顺手。 */
function trendChart(
  items: TrendItem[],
  colors: Map<string, string>,
  dur: (secs: number) => string,
): string {
  const withData = items.filter((i) => i.totalSecs > 0);
  if (withData.length === 0) return "";
  const max = Math.max(...withData.map((i) => i.totalSecs));
  const H = 120;
  // 柱多时抽稀标签，避免 90 天标签挤成一团；柱顶小时数同理
  const maxLabels = 10;
  const labelStep = Math.max(1, Math.ceil(items.length / maxLabels));
  // 周 / 月只有几根柱，全宽拉伸会很肥，给固定最大宽度并居中
  const sparse = items.length <= 7;
  // 柱顶数值：20 根以内全标；更多时抽稀，但**最高那根必须有数**——之前抽稀
  // 是固定步长，跟柱子高矮无关，全场最高的柱恰好落在步长缝里没数字，矮它
  // 一头的反倒标着 11.3h，怎么看怎么不齐。峰值旁紧贴的常规标签让位，防重叠。
  const maxIdx = items.reduce((bi, it, i) => (it.totalSecs > items[bi].totalSecs ? i : bi), 0);
  const valueStep = items.length <= 20 ? 1 : Math.ceil(items.length / 13);
  const showValue = (i: number): boolean =>
    i === maxIdx || (i % valueStep === 0 && Math.abs(i - maxIdx) > 1);

  const cols = items
    .map((it, i) => {
      // 整柱高度按总时长精确算，段高是柱内占比。之前每段各给 2px 下限再留 1px 缝，
      // 细分类多的柱被硬生生垫高，2h 的柱和 8h 的肩并肩，数值标签也全悬在同一高度
      // ——用户两轮截图圈的"高低不齐"就是这里。
      const hPx = Math.max((it.totalSecs / max) * H, 2);
      const stack = it.categories
        .filter((c) => c.secs > 0)
        .map((c) => {
          const pct = (c.secs / it.totalSecs) * 100;
          const color = htmlEscape(colors.get(c.id) ?? "#94a3b8");
          return `<i style="height:${pct.toFixed(2)}%;background:${color}"></i>`;
        })
        .join("");
      const shown = i % labelStep === 0;
      const value = showValue(i)
        ? `<span class="vlabel">${it.totalSecs > 0 ? htmlEscape(shortHours(it.totalSecs)) : ""}</span>`
        : "";
      // 悬停明细走页尾脚本画的浮层：日期 + 总计 + 分类拆分，
      // 不再用浏览器原生 title 那个裸框
      const tipLines = [
        `${it.label} · ${dur(it.totalSecs)}`,
        ...it.categories
          .filter((c) => c.secs > 0)
          .map((c) => `${c.name}  ${dur(c.secs)} · ${Math.round((c.secs / it.totalSecs) * 100)}%`),
      ].join("\n");
      return (
        `<a class="col" href="#${htmlEscape(it.anchor)}" ` +
        `data-tip="${htmlEscape(tipLines)}">` +
        `${value}<div class="stack${stack ? "" : " stack-empty"}"${stack ? ` style="height:${hPx.toFixed(1)}px"` : ""}>${stack}</div>` +
        `<span class="tlabel">${shown ? htmlEscape(it.shortLabel) : ""}</span>` +
        `</a>`
      );
    })
    .join("");
  return `<div class="trend"><div class="trend-inner${sparse ? " sparse" : ""}">${cols}</div></div>`;
}

/** 报告样式：全部内联，离线可读；配色 / 圆角 / 阴影对齐 Hindsight 应用本体
 *  （src/styles/tokens.css），深浅色跟随系统，打印时收掉底色强制浅色。 */
const HTML_STYLE = `
:root{--bg:#fafafa;--card:#fbfbfd;--open:#f4f4f8;--text:#1d1c25;--text-2:#2e2c3a;--muted:#6b6680;--faint:#9a96aa;
 --line:rgba(0,0,0,.08);--line-subtle:rgba(0,0,0,.05);--accent:#6c5ce7;--accent-soft:rgba(108,92,231,.12);
 --chip:rgba(255,255,255,.6);--nav:rgba(250,250,250,.88);
 --shadow:0 1px 2px rgba(20,20,40,.04),0 8px 20px rgba(20,20,40,.06)}
 @media(prefers-color-scheme:dark){:root{--bg:#0e0e11;--card:#1d1d23;--open:#24242c;--text:#dadae1;--text-2:#c6c6d0;
 --muted:#9b9ba6;--faint:#6c6c78;--line:rgba(255,255,255,.1);--line-subtle:rgba(255,255,255,.06);
 --accent:#8b7bf0;--accent-soft:rgba(139,123,240,.18);--chip:rgba(255,255,255,.05);--nav:rgba(14,14,17,.88);
 --shadow:0 1px 2px rgba(0,0,0,.3),0 8px 20px rgba(0,0,0,.35)}
 .app-icon{background-color:#27272f}}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%;scroll-behavior:smooth}
body{margin:0;background:var(--bg);color:var(--text);
 font:14px/1.6 "Inter",-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,"Helvetica Neue",Arial,"PingFang SC","Microsoft YaHei",sans-serif;
 -webkit-font-smoothing:antialiased}
a{color:inherit;text-decoration:none}
.wrap{max-width:820px;margin:0 auto;padding:26px 24px 72px}
/* 吸顶条会盖住锚点目标的头部，给所有跳转目标留出高度 */
[id]{scroll-margin-top:64px}
/* 吸顶导航：任何位置一键换章，不用往回滚 */
.nav{position:sticky;top:0;z-index:9;background:var(--nav);
 backdrop-filter:saturate(1.5) blur(14px);-webkit-backdrop-filter:saturate(1.5) blur(14px);
 border-bottom:1px solid var(--line)}
.nav-in{max-width:820px;margin:0 auto;padding:8px 24px;display:flex;align-items:center;gap:7px;flex-wrap:wrap}
.nav-brand{display:flex;align-items:center;gap:7px;font-size:12.5px;font-weight:650;color:var(--text);margin-right:auto}
.nav-brand svg{width:19px;height:19px}
.nlink{font-size:12px;color:var(--muted);border:1px solid var(--line-subtle);background:var(--chip);
 border-radius:999px;padding:4px 12px;transition:color .15s,border-color .15s,background-color .15s}
.nlink:hover,.nlink.on{color:var(--accent);border-color:var(--accent);background:var(--accent-soft)}
/* 页头 */
.head{display:flex;flex-direction:column;gap:11px;margin-top:14px}
.brand{display:flex;align-items:center;gap:9px}
.brand svg{width:26px;height:26px;border-radius:8px;box-shadow:0 2px 8px rgba(108,92,231,.25)}
.brand-name{font-size:15px;font-weight:650;letter-spacing:-.01em;color:var(--text)}
h1{font-size:23px;font-weight:650;margin:0;letter-spacing:-.015em;line-height:1.3}
.metas{display:flex;flex-wrap:wrap;gap:8px}
.chip{display:inline-flex;align-items:center;gap:6px;font-size:12px;color:var(--muted);
 background:var(--chip);border:1px solid var(--line-subtle);border-radius:999px;padding:4px 11px}
.chip .cd{width:6px;height:6px;border-radius:50%;background:var(--accent)}
.note{font-size:12px;color:var(--faint);margin:0;max-width:56em}
/* 总览：打开先看见整段范围的结论，再往下才是拆解 */
.hero{background:var(--card);border:1px solid var(--line);border-radius:14px;box-shadow:var(--shadow);
 padding:18px 20px 16px;margin-top:20px}
.hero-top{display:flex;align-items:baseline;gap:10px;flex-wrap:wrap;margin-bottom:2px}
.hero-lab{font-size:12px;color:var(--faint)}
.hero-num{font-size:29px;font-weight:680;letter-spacing:-.025em;font-variant-numeric:tabular-nums;line-height:1.1}
.hero-avg{font-size:12.5px;color:var(--muted);margin-left:auto;font-variant-numeric:tabular-nums}
.hero .sharebar{margin-top:14px;height:10px;border-radius:5px}
/* 图例 */
.legend{display:flex;flex-wrap:wrap;gap:6px 8px;margin-top:14px}
.legend .item{display:inline-flex;align-items:center;gap:6px;font-size:12px;color:var(--muted);
 background:var(--chip);border:1px solid var(--line-subtle);border-radius:999px;padding:3px 10px}
.dot{display:inline-block;width:8px;height:8px;border-radius:50%;flex-shrink:0}
/* 章节 */
.section{margin-top:38px}
.section-head{display:flex;align-items:baseline;justify-content:space-between;gap:12px;
 padding-bottom:10px;border-bottom:1px solid var(--line)}
.section-head h2{font-size:15px;font-weight:650;margin:0;letter-spacing:-.01em}
.section-head .sub{font-size:11.5px;color:var(--faint)}
/* 「全部展开/收起」由脚本注入——没有脚本的环境根本不出现死按钮 */
.xall{font-size:11.5px;color:var(--muted);background:var(--chip);border:1px solid var(--line-subtle);
 border-radius:999px;padding:3px 10px;cursor:pointer;font-family:inherit;
 transition:color .15s,border-color .15s}
.xall:hover{color:var(--accent);border-color:var(--accent)}
/* 跳转胶囊：这一章有哪些周期，一眼看全，点哪去哪 */
.jump{display:flex;flex-wrap:wrap;gap:5px;margin-top:12px}
.jump a{font-size:11.5px;font-variant-numeric:tabular-nums;color:var(--muted);
 border:1px solid var(--line-subtle);background:var(--chip);border-radius:7px;padding:3px 8px;
 transition:color .15s,border-color .15s,background-color .15s}
.jump a:hover{color:var(--accent);border-color:var(--accent);background:var(--accent-soft)}
.jump a.zero{color:var(--faint);opacity:.5}
/* 趋势柱状图 */
.trend{margin-top:16px}
.trend-inner{display:flex;align-items:stretch;gap:4px}
.trend-inner.sparse{justify-content:center}
.trend-inner.sparse .col{flex:0 1 100px}
.col{flex:1;display:flex;flex-direction:column;justify-content:flex-end;gap:5px;min-width:0}
.col:hover .stack{outline:1.5px solid var(--accent);outline-offset:1px}
.col:hover .tlabel,.col:hover .vlabel{color:var(--accent)}
.vlabel{font-size:9.5px;color:var(--faint);text-align:center;white-space:nowrap;line-height:1.1;
 font-variant-numeric:tabular-nums}
/* 柱高由内联 style 按数据给；数值标签因此贴着柱顶随柱走，不再悬在统一高度 */
.stack{display:flex;flex-direction:column-reverse;justify-content:flex-start;
 border-bottom:1px solid var(--line-subtle);border-radius:3px 3px 0 0;overflow:hidden}
.stack-empty{height:3px;background:transparent;border-bottom:1px solid var(--line)}
.stack i{display:block;width:100%}
/* 柱多时日期标签抽稀（每 N 根一个），左右都空着 —— 允许溢出，
   否则单根柱宽放不下「08-01」，会截成没用的「08-…」。
   height 必须定死：空标签的高度会塌陷成 0，那根柱就比带日期的沉 15px，
   26 根柱每 3 根一浮一沉，整排锯齿 —— 用户说的"歪歪扭扭"就是它。 */
.tlabel{font-size:10px;color:var(--faint);text-align:center;white-space:nowrap;
 overflow:visible;font-variant-numeric:tabular-nums;height:15px}
/* 周期列表：一个周期一行，默认折叠，点开才展细节。
   26 天各占一整屏卡片时，想找某天只能一路滚；收成行，一屏半就扫完了。
   这也是对齐的根：所有行共用一套 grid 列宽，数字全落在同一竖线上。 */
.rows{margin-top:14px;background:var(--card);border:1px solid var(--line);border-radius:12px;
 box-shadow:var(--shadow);overflow:hidden}
.pd{position:relative}
.pd+.pd{border-top:1px solid var(--line-subtle)}
/* 展开靠 checkbox 而不是 details 元素：后者的折叠内容藏在浏览器自己的
   shadow DOM 里，作者样式够不着，打印时摊不平（实测 Ctrl+P 只出汇总行）。
   checkbox 的显隐是普通 CSS，@media print 一句话就能全摊开。 */
.tgl{position:absolute;width:1px;height:1px;opacity:0;margin:0;pointer-events:none}
.row{display:grid;grid-template-columns:20px 1fr 104px 176px;align-items:center;gap:12px;
 padding:10px 14px;cursor:pointer;transition:background-color .12s}
.row:hover{background:var(--accent-soft)}
.row::before{content:"";justify-self:center;width:0;height:0;
 border-left:4.5px solid var(--faint);border-top:4px solid transparent;border-bottom:4px solid transparent;
 transition:transform .15s}
.det{display:none}
.tgl:checked~.det{display:block;background:var(--open)}
.tgl:checked~.row{background:var(--open)}
.tgl:checked~.row::before{transform:rotate(90deg)}
.tgl:focus-visible~.row{outline:2px solid var(--accent);outline-offset:-2px}
/* 从趋势柱 / 胶囊跳过来的那一行标记出来，落地就知道停在哪 */
.pd:target>.row{box-shadow:inset 3px 0 0 var(--accent)}
.ttl{font-size:13.5px;font-weight:600;letter-spacing:-.01em;font-variant-numeric:tabular-nums;
 overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.ttl em{font-style:normal;font-weight:400;color:var(--muted);margin-left:5px;font-size:12.5px}
.row .tot{font-size:13.5px;font-weight:650;text-align:right;font-variant-numeric:tabular-nums;
 letter-spacing:-.01em;white-space:nowrap}
.row.zero .tot,.row.zero .ttl{color:var(--faint);font-weight:500}
.sharebar{display:flex;height:8px;border-radius:4px;overflow:hidden;background:var(--line-subtle);width:100%}
.sharebar i{display:block;height:100%}
.empty-bar{opacity:.5}
/* 展开区：日均 + 分类表 + 应用表 */
.det{padding:2px 14px 14px 28px}
.sub-head{font-size:12.5px;font-weight:650;margin:16px 0 2px;padding-left:8px;letter-spacing:-.01em;color:var(--text-2)}
/* table-layout:fixed —— 列宽由 col 定死，不随内容浮动。auto 布局下每张表各算各的，
   几十个周期叠起来就是一片参差（两张表的「时长」列尤其明显）。 */
table{width:100%;border-collapse:collapse;font-size:13px;table-layout:fixed}
th{padding:6px 8px;text-align:left;font-size:11px;font-weight:600;color:var(--faint);
 border-bottom:1px solid var(--line-subtle);letter-spacing:.02em}
td{padding:6px 8px;border-bottom:1px solid var(--line-subtle);vertical-align:middle;
 overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
tr:last-child td{border-bottom:none}
td.n,th.n{text-align:right;font-variant-numeric:tabular-nums;color:var(--text-2)}
td.share{color:var(--muted)}
.cat-name{display:inline-flex;align-items:center;gap:8px;color:var(--text);min-width:0}
.empty{color:var(--faint);font-size:13px;padding:2px 8px 8px}
/* 应用行：图标 + 名称 + 分类 chip */
.app-cell{display:flex;align-items:center;gap:10px;min-width:0;color:var(--text)}
.app-name{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.app-icon{width:24px;height:24px;border-radius:7px;flex-shrink:0;
 box-shadow:0 0 0 .5px var(--line);background:#fff center/19px 19px no-repeat}
.app-fallback{display:inline-flex;align-items:center;justify-content:center;
 font-size:11.5px;font-weight:650;box-shadow:0 0 0 .5px var(--line)}
.cat{display:inline-flex;align-items:center;gap:6px;font-size:12px;color:var(--muted);white-space:nowrap}
/* 悬停浮层：内容由脚本填 textContent，pre-line 保留换行 */
.tip{position:fixed;z-index:99;display:none;max-width:280px;background:var(--card);color:var(--text);
 border:1px solid var(--line);border-radius:10px;box-shadow:var(--shadow);padding:8px 12px;
 font-size:12px;line-height:1.65;pointer-events:none;white-space:pre-line;
 font-variant-numeric:tabular-nums}
/* 页脚 */
.report-foot{margin-top:44px;padding-top:14px;border-top:1px solid var(--line);
 display:flex;align-items:center;justify-content:space-between;gap:16px;flex-wrap:wrap;
 font-size:11.5px;color:var(--faint)}
.report-foot .brand{font-size:12.5px;color:var(--muted)}
/* 窄屏：占比条这一列先让位，标题和时长不能丢 */
@media(max-width:600px){.row{grid-template-columns:20px 1fr 96px}.row .sharebar{display:none}
 .wrap,.nav-in{padding-left:16px;padding-right:16px}.det{padding-left:14px}}
@media print{@page{margin:14mm}
:root{--bg:#fff;--card:#fff;--open:#fff;--text:#1d1c25;--text-2:#2e2c3a;--muted:#6b6680;--faint:#9a96aa;
 --line:rgba(0,0,0,.1);--line-subtle:rgba(0,0,0,.06);--chip:#fff;--shadow:none}
body{background:#fff}.wrap{max-width:none;padding:0}
.nav,.jump,.xall,.tip{display:none}
/* 纸上点不开：强制摊平所有折叠块（UA 用 content-visibility 藏起来，一并覆盖） */
.det{display:block}
.tgl,.row::before{display:none}
.rows{box-shadow:none;border-radius:0}
.pd{break-inside:avoid}.hero{break-inside:avoid;box-shadow:none}
.tgl:checked~.det,.tgl:checked~.row{background:#fff}
.app-icon{background-color:#fff}}
`.trim();

/** 进程名 → CSS 类名（`ic0`、`ic1`…），只给真拿到图的应用发号。 */
function iconClassMap(icons: Map<string, string>): Map<string, string> {
  const m = new Map<string, string>();
  let i = 0;
  for (const [name, url] of icons) if (url) m.set(name, `ic${i++}`);
  return m;
}

/** 图标的 CSS 规则：每张图的 base64 **只出现一次**。
 *
 *  之前是每个单元格一个 `<img src="data:...">`——同一个应用在 26 天 × 日/周/月
 *  里要出现上百回，base64 就跟着复制上百份，实测把一份月报顶到 32 MB。改成
 *  发一个类名去引用，重复的地方只剩 `ic3` 三个字符。 */
function iconStyleRules(icons: Map<string, string>, classes: Map<string, string>): string {
  const out: string[] = [];
  for (const [name, cls] of classes) {
    const url = icons.get(name);
    if (url) out.push(`.${cls}{background-image:url("${url.replace(/"/g, "%22")}")}`);
  }
  return out.join("\n");
}

/** 应用图标单元格：有图就用它的类，没有就分类色圆角块 + 首字符。 */
function appIconHtml(a: AppStat, colors: Map<string, string>, classes: Map<string, string>): string {
  const color = colors.get(a.categoryId) ?? "#94a3b8";
  const cls = classes.get(a.iconProcess ?? "");
  if (cls) return `<span class="app-icon ${cls}" aria-hidden="true"></span>`;
  const first = [...a.name.trim()][0] ?? "?";
  return (
    `<span class="app-icon app-fallback" aria-hidden="true" ` +
    `style="background:color-mix(in srgb, ${htmlEscape(color)} 15%, transparent);color:${htmlEscape(color)}">` +
    `${htmlEscape(first.toUpperCase())}</span>`
  );
}

/** 表格列宽：两张表都把「时长」放最后一列且同宽，右边缘因此落在同一竖线上。 */
const CAT_COLS = `<colgroup><col><col style="width:62px"><col style="width:104px"></colgroup>`;
const APP_COLS =
  `<colgroup><col style="width:36px"><col><col style="width:120px">` +
  `<col style="width:104px"></colgroup>`;

/** 把多个周期的分类用时并成一份（按秒累加后重排）。
 *  秒是真值、minutes 只是显示：逐桶取整再相加会系统性偏小，所以求和只认 secs。 */
function mergeCats(items: { categories: CatStat[] }[]): CatStat[] {
  const m = new Map<string, CatStat>();
  for (const it of items) {
    for (const c of it.categories) {
      const cur = m.get(c.id);
      if (cur) cur.secs += c.secs;
      else m.set(c.id, { ...c });
    }
  }
  const out = [...m.values()];
  for (const c of out) c.minutes = Math.round(c.secs / 60);
  return out.sort((a, b) => b.secs - a.secs);
}

/** 一个周期 = 列表里的一行，点开才展细节（checkbox + label，不用一行脚本）。
 *
 *  收起时只留「标题 · 总时长 · 占比条」——一个月 26 天也就一屏半，横着一比就知道
 *  哪天异常；想看某天的分类和应用，点开就地展开，不用离开这份列表。 */
function periodRowHtml(
  heading: string,
  stat: { totalSecs: number; categories: CatStat[]; apps: AppStat[]; dailyAvgSecs?: number | null },
  anchor: string,
  labels: UsageExportLabels,
  colors: Map<string, string>,
  classes: Map<string, string>,
  open: boolean,
): string {
  const { t, fmtDuration } = labels;
  const dur = (secs: number): string => fmtDuration(Math.round(secs / 60));
  const zero = stat.totalSecs <= 0;
  const avg =
    stat.dailyAvgSecs != null && !zero
      ? `<em>${htmlEscape(t("settings.data.export.file.dailyAvg"))} ${htmlEscape(dur(stat.dailyAvgSecs))}</em>`
      : "";

  const id = htmlEscape(anchor);
  const out: string[] = [
    `<div class="pd" id="${id}">`,
    `<input class="tgl" type="checkbox" id="t-${id}"${open ? " checked" : ""}>`,
    `<label class="row${zero ? " zero" : ""}" for="t-${id}">`,
    `<span class="ttl">${htmlEscape(heading)}${avg}</span>`,
    `<span class="tot">${htmlEscape(dur(stat.totalSecs))}</span>`,
    shareBar(stat.categories, stat.totalSecs, colors, dur),
    `</label><div class="det">`,
  ];

  if (zero) {
    out.push(`<p class="empty">${htmlEscape(t("settings.data.export.file.noData"))}</p></div></div>`);
    return out.join("");
  }

  out.push(
    `<table>${CAT_COLS}<thead><tr>` +
      `<th>${htmlEscape(t("settings.data.export.file.colCategory"))}</th>` +
      `<th class="n">${htmlEscape(t("settings.data.export.file.colShare"))}</th>` +
      `<th class="n">${htmlEscape(t("settings.data.export.file.colDuration"))}</th>` +
      `</tr></thead><tbody>`,
  );
  for (const c of stat.categories) {
    const share = Math.round((c.secs / stat.totalSecs) * 100);
    const dot = `<span class="dot" style="background:${htmlEscape(colors.get(c.id) ?? "#94a3b8")}"></span>`;
    out.push(
      `<tr><td><span class="cat-name">${dot}${htmlEscape(c.name)}</span></td>` +
        `<td class="n share">${share}%</td><td class="n">${htmlEscape(dur(c.secs))}</td></tr>`,
    );
  }
  out.push(`</tbody></table>`);

  if (stat.apps.length > 0) {
    out.push(
      `<h4 class="sub-head">${htmlEscape(t("settings.data.export.file.topAppsTitle", { n: MARKDOWN_TOP_APPS }))}</h4>`,
      `<table>${APP_COLS}<thead><tr><th class="n">#</th>` +
        `<th>${htmlEscape(t("settings.data.export.file.colApp"))}</th>` +
        `<th>${htmlEscape(t("settings.data.export.file.colCategory"))}</th>` +
        `<th class="n">${htmlEscape(t("settings.data.export.file.colDuration"))}</th>` +
        `</tr></thead><tbody>`,
    );
    stat.apps.slice(0, MARKDOWN_TOP_APPS).forEach((a, i) => {
      const catDot = `<span class="dot" style="background:${htmlEscape(colors.get(a.categoryId) ?? "#94a3b8")}"></span>`;
      out.push(
        `<tr><td class="n">${i + 1}</td>` +
          `<td><span class="app-cell">${appIconHtml(a, colors, classes)}` +
          `<span class="app-name">${htmlEscape(a.name)}</span></span></td>` +
          `<td><span class="cat">${catDot}${htmlEscape(a.categoryName)}</span></td>` +
          `<td class="n">${htmlEscape(fmtDuration(a.minutes))}</td></tr>`,
      );
    });
    out.push(`</tbody></table>`);
  }

  out.push(`</div></div>`);
  return out.join("");
}

/** Hindsight 品牌标记（橙→紫渐变圆角块 + 眼睛剪影，与应用 logo 同款配色）。
 *  同一页要画两次（吸顶条 + 页头），渐变 id 得各用各的，否则两个 defs 撞名。 */
function brandMark(id: string): string {
  return `
<svg width="26" height="26" viewBox="0 0 26 26" aria-hidden="true">
<defs><linearGradient id="${id}" x1="0" y1="0" x2="1" y2="1">
<stop offset="0" stop-color="#ff8a5c"/><stop offset="1" stop-color="#6c5ce7"/></linearGradient></defs>
<rect x="0.75" y="0.75" width="24.5" height="24.5" rx="7.5" fill="url(#${id})"/>
<path d="M5.6 13.1c2.5-3.3 5.6-5 7.4-5s4.9 1.7 7.4 5c-2.5 3.3-5.6 5-7.4 5s-4.9-1.7-7.4-5Z" fill="none" stroke="#fff" stroke-width="1.5" stroke-linecap="round"/>
<circle cx="13" cy="13.1" r="2.3" fill="#fff"/>
</svg>`.trim();
}

/** 一个章节要渲染的东西：胶囊 + 趋势图 + 折叠行都从这里来。 */
interface Section {
  id: string;
  label: string;
  items: TrendItem[];
  rows: { heading: string; anchor: string; stat: DayStat | PeriodStat }[];
  /** 折叠行是否默认展开（月份只有一两个，摊开更省事）。 */
  open: boolean;
  /** 章节内是否再画一张趋势图（总览已经画了最细的那张，别重复）。 */
  trend: boolean;
}

function renderHtml(
  data: UsageExportData,
  labels: UsageExportLabels,
  icons: Map<string, string> = new Map(),
): string {
  const { t, locale } = labels;
  const colors = catColorMap(data);
  const classes = iconClassMap(icons);
  const dur = (secs: number): string => labels.fmtDuration(Math.round(secs / 60));
  const weekdayFmt = new Intl.DateTimeFormat(locale, { weekday: "short" });
  const title = t("settings.data.export.file.title", {
    start: data.rangeStart,
    end: data.rangeEnd,
  });

  // —— 章节按「总 → 分」排：月、周、日。打开先看结论，越往下越细 ——
  const sections: Section[] = [];

  if (data.monthly) {
    sections.push({
      id: "monthly",
      label: t("settings.data.export.file.monthlyHeading"),
      items: data.monthly.map((p) => ({
        label: `${p.start} ~ ${p.end}`,
        shortLabel: p.start.slice(0, 7),
        totalSecs: p.totalSecs,
        categories: p.categories,
        anchor: `m-${p.start.slice(0, 7)}`,
      })),
      rows: data.monthly.map((p) => ({
        heading: t("settings.data.export.file.monthHeading", {
          month: p.start.slice(0, 7),
          start: p.start,
          end: p.end,
        }),
        anchor: `m-${p.start.slice(0, 7)}`,
        stat: p,
      })),
      open: data.monthly.length <= 2,
      trend: data.monthly.length >= 2,
    });
  }

  if (data.weekly) {
    sections.push({
      id: "weekly",
      label: t("settings.data.export.file.weeklyHeading"),
      items: data.weekly.map((p) => ({
        label: `${p.start} ~ ${p.end}`,
        shortLabel: `${p.start.slice(5)} ~ ${p.end.slice(5)}`,
        totalSecs: p.totalSecs,
        categories: p.categories,
        anchor: `w-${p.start}`,
      })),
      rows: data.weekly.map((p) => ({
        heading: t("settings.data.export.file.weekHeading", { start: p.start, end: p.end }),
        anchor: `w-${p.start}`,
        stat: p,
      })),
      open: false,
      trend: true,
    });
  }

  if (data.daily) {
    sections.push({
      id: "daily",
      label: t("settings.data.export.file.dailyHeading"),
      items: data.daily.map((d) => ({
        label: d.date,
        shortLabel: d.date.slice(5),
        totalSecs: d.totalSecs,
        categories: d.categories,
        anchor: `d-${d.date}`,
      })),
      rows: data.daily.map((d) => {
        const [y, m, dd] = d.date.split("-").map((v) => parseInt(v, 10));
        return {
          heading: t("settings.data.export.file.dayHeading", {
            date: d.date,
            weekday: weekdayFmt.format(new Date(y, m - 1, dd)),
          }),
          anchor: `d-${d.date}`,
          stat: d,
        };
      }),
      open: data.daily.length === 1,
      // 总览里那张趋势图画的就是日粒度，这儿不再重复一遍
      trend: false,
    });
  }

  const navLinks = sections
    .map((s) => `<a class="nlink" href="#${s.id}">${htmlEscape(s.label)}</a>`)
    .join("");

  const body: string[] = [
    `<nav class="nav"><div class="nav-in">`,
    `<a class="nav-brand" href="#top">${brandMark("hs-nav")}<span>Hindsight</span></a>`,
    navLinks,
    `</div></nav>`,
    `<div class="wrap" id="top">`,
    `<header class="head">`,
    `<div class="brand">${brandMark("hs-head")}<span class="brand-name">Hindsight</span></div>`,
    `<h1>${htmlEscape(title)}</h1>`,
    `<div class="metas">`,
    `<span class="chip"><span class="cd"></span>${htmlEscape(t("settings.data.export.file.metaRange", { start: data.rangeStart, end: data.rangeEnd }))}</span>`,
    `<span class="chip">${htmlEscape(t("settings.data.export.file.metaExportedAt", { time: new Date(data.exportedAt).toLocaleString(locale) }))}</span>`,
    `</div>`,
    `<p class="note">${htmlEscape(t("settings.data.export.file.metaNote", { n: MARKDOWN_TOP_APPS }))}</p>`,
    `</header>`,
  ];

  // 分类图例：一次说清后面所有色块的含义，省得每张图各配一份
  const usedCats = new Map<string, string>();
  for (const src of [data.daily ?? [], data.weekly ?? [], data.monthly ?? []]) {
    for (const p of src) for (const c of p.categories) if (c.secs > 0) usedCats.set(c.id, c.name);
  }
  const legend =
    usedCats.size > 0
      ? `<div class="legend">` +
        [...usedCats.entries()]
          .map(
            ([id, name]) =>
              `<span class="item"><span class="dot" style="background:${htmlEscape(colors.get(id) ?? "#94a3b8")}"></span>${htmlEscape(name)}</span>`,
          )
          .join("") +
        `</div>`
      : "";

  // —— 总览：整段范围的总时长 / 占比 / 趋势 / 图例，一屏之内 ——
  //    只有勾了「每日」才算得出范围总计：周和月会超出所选范围、还彼此重叠，不能相加。
  if (data.daily) {
    const totalSecs = data.daily.reduce((s, d) => s + d.totalSecs, 0);
    // 日均按「已完成的天」算，今天只过了一半不该把均值拖下来（与周/月卡片同口径）
    const exportDay = fmtLocalDate(new Date(data.exportedAt));
    const done = data.daily.filter((d) => d.date < exportDay).length;
    const avg =
      done > 0
        ? `<span class="hero-avg">${htmlEscape(t("settings.data.export.file.dailyAvg"))} ` +
          `${htmlEscape(dur(totalSecs / done))}</span>`
        : "";
    body.push(
      `<div class="hero"><div class="hero-top">`,
      `<span class="hero-lab">${htmlEscape(t("settings.data.export.file.total"))}</span>`,
      avg,
      `</div>`,
      `<div class="hero-num">${htmlEscape(dur(totalSecs))}</div>`,
      shareBar(mergeCats(data.daily), totalSecs, colors, dur),
      trendChart(sections.find((s) => s.id === "daily")?.items ?? [], colors, dur),
      legend,
      `</div>`,
    );
  } else if (legend) {
    body.push(legend);
  }

  for (const s of sections) {
    body.push(
      `<section class="section" id="${s.id}"><div class="section-head">` +
        `<h2>${htmlEscape(s.label)}</h2></div>`,
    );
    if (s.items.length > 0) {
      body.push(
        `<div class="jump">` +
          s.items
            .map(
              (it) =>
                `<a href="#${htmlEscape(it.anchor)}"${it.totalSecs > 0 ? "" : ' class="zero"'} ` +
                `data-tip="${htmlEscape(`${it.label} · ${dur(it.totalSecs)}`)}">` +
                `${htmlEscape(it.shortLabel)}</a>`,
            )
            .join("") +
          `</div>`,
      );
    }
    if (s.trend) body.push(trendChart(s.items, colors, dur));
    body.push(`<div class="rows">`);
    for (const r of s.rows) {
      body.push(periodRowHtml(r.heading, r.stat, r.anchor, labels, colors, classes, s.open));
    }
    body.push(`</div></section>`);
  }

  body.push(
    `<footer class="report-foot">` +
      `<span>${htmlEscape(t("settings.data.export.file.metaExportedAt", { time: new Date(data.exportedAt).toLocaleString(locale) }))}</span>` +
      `<span class="brand"><span class="dot" style="background:var(--accent)"></span>Hindsight</span>` +
      `</footer>`,
    `</div>`,
  );

  const expandAll = JSON.stringify(t("settings.data.export.file.expandAll"));
  const collapseAll = JSON.stringify(t("settings.data.export.file.collapseAll"));
  // 交互增强，全部内联（约 1.5KB）：锚点跳到折叠行时自动展开、吸顶导航高亮当前章、
  // 每章一键展开/收起。纯事件驱动 + 一个 IntersectionObserver，零轮询零定时器，
  // 报告再长也不吃 CPU；没有脚本的环境（老阅读器、部分邮件客户端）报告照样能读
  // ——折叠是 checkbox、导航是锚点，脚本只是让它们更顺手。
  const script =
    `<script>(function(){"use strict";` +
    `var EXPAND=${expandAll},COLLAPSE=${collapseAll};` +
    // 胶囊 / 趋势柱 / 手输 hash 跳到某行时把它点开，不然落地还是一条收着的行
    `function openHash(){var id=location.hash.slice(1);if(!id)return;` +
    `var el=document.getElementById(id);if(!el)return;` +
    `var t=el.querySelector(".tgl");if(t)t.checked=true}` +
    `addEventListener("hashchange",openHash);openHash();` +
    `var links={};[].forEach.call(document.querySelectorAll(".nav .nlink"),` +
    `function(a){links[a.getAttribute("href").slice(1)]=a});` +
    `var io=new IntersectionObserver(function(es){es.forEach(function(e){` +
    `var a=links[e.target.id];if(a)a.classList.toggle("on",e.isIntersecting)})},` +
    `{rootMargin:"-64px 0px -70% 0px"});` +
    `[].forEach.call(document.querySelectorAll("section.section"),function(s){io.observe(s);` +
    `var tgls=s.querySelectorAll(".tgl");if(!tgls.length)return;` +
    `var b=document.createElement("button");b.type="button";b.className="xall";var open=false;` +
    `function refresh(){open=false;for(var i=0;i<tgls.length;i++)if(tgls[i].checked){open=true;break}` +
    `b.textContent=open?COLLAPSE:EXPAND}refresh();` +
    `b.addEventListener("click",function(){var to=!open;` +
    `for(var i=0;i<tgls.length;i++)tgls[i].checked=to;refresh()});` +
    `s.addEventListener("change",refresh);` +
    `s.querySelector(".section-head").appendChild(b)});` +
    // 自绘 tooltip:一个 mouseover 委托 + 进入时定位一次,不跟鼠标、零 mousemove,
    // 页面再长也不吃 CPU。内容走 textContent,data-tip 里的用户数据不会被当标签解析。
    `var tip=document.createElement("div");tip.className="tip";document.body.appendChild(tip);` +
    `function hideTip(){tip.style.display="none"}` +
    `document.addEventListener("mouseover",function(e){` +
    `var el=e.target&&e.target.closest?e.target.closest("[data-tip]"):null;` +
    `if(!el)return hideTip();` +
    `tip.textContent=el.getAttribute("data-tip");tip.style.display="block";` +
    `var r=el.getBoundingClientRect(),tw=tip.offsetWidth,th=tip.offsetHeight;` +
    `var x=Math.max(8,Math.min(r.left+r.width/2-tw/2,innerWidth-tw-8));` +
    `var y=r.top-th-8;if(y<8)y=r.bottom+8;` +
    `tip.style.left=x+"px";tip.style.top=y+"px"});` +
    `addEventListener("scroll",hideTip,{passive:true})` +
    `})()</script>`;

  const iconCss = iconStyleRules(icons, classes);
  return [
    `<!doctype html>`,
    `<html lang="${htmlEscape(locale)}">`,
    `<head>`,
    `<meta charset="utf-8">`,
    `<meta name="viewport" content="width=device-width,initial-scale=1">`,
    `<title>${htmlEscape(title)}</title>`,
    `<style>${HTML_STYLE}${iconCss ? `\n${iconCss}` : ""}</style>`,
    `</head>`,
    `<body>${body.join("")}${script}</body>`,
    `</html>`,
    "",
  ].join("\n");
}
