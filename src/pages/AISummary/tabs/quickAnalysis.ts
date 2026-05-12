/**
 * 快速模板的"伪 AI 分析"段落生成。
 *
 * 思路：完全在前端跑决策树，根据 quick summary 的数据指标挑一组 i18n 预设句子拼起来。
 * 不调任何模型，每个分支都是事先写好的固定句式 + 占位符。三语文案都在 i18n 里，本文件
 * 只做"分支选择 + 填占位"。
 *
 * 决策树的优先级是"信号越强越优先"——比如凌晨时段 > 60min 比"应用切换频繁"更值得提醒，
 * 因为前者是健康相关的直接信号，后者只是效率层面的弱信号。
 *
 * 分类感知：识别"工作类"（code/browse/talk/design）vs "娱乐类"（fun）的占比，
 * 在 workOverview / efficiency / suggestion / summary 各段落都参考这个信号选不同分支。
 * 非内置分类（'other' / 用户自建 UUID）不参与"工作 vs 娱乐"对比，但仍会被点名为"主导分类"。
 */

import type {
  QuickDaySummary,
  QuickMonthSummary,
  QuickUsageEntry,
  QuickWeekSummary,
} from "../../../api/hindsight";

type T = (key: string, opts?: Record<string, unknown>) => string;
type CatName = (id: string) => string;

/** 分析结果——每段任意 null（前端按非空渲染）。
 *  段落顺序：概述 → 时间节奏 → 分类构成 → 亮点 → 专注度 → 效率 → 建议 → 小结。 */
export interface QuickAnalysis {
  workOverview: string | null;
  timePattern: string | null;
  categoryBreakdown: string | null;
  highlights: string | null;
  focus: string | null;
  efficiency: string | null;
  suggestion: string | null;
  summary: string | null;
}

const NIGHT_HEAVY_MINUTES = 60; // 凌晨段超过 1h 触发"作息"建议
const HEAVY_DAY_MINUTES = 360; // 单日 > 6h 算"饱满"
const BALANCED_DAY_MINUTES = 180; // 3-6h 算"稳定"
const LIGHT_DAY_MINUTES = 60; // 1-3h 算"轻松"，<1h 算"极少"

/** 内置工作类分类——code/browse/talk/design 都视为生产类。 */
const WORK_LIKE_CATS = new Set(["code", "browse", "talk", "design"]);
/** 内置娱乐类分类。 */
const FUN_LIKE_CATS = new Set(["fun"]);
/** 主导分类的"显著"门槛：占比超过这个才被点名。 */
const PRIMARY_CATEGORY_THRESHOLD = 0.25;
/** 娱乐占比偏高的门槛。 */
const FUN_HEAVY_THRESHOLD = 0.35;
/** 工作 vs 娱乐"接近均衡"的判定：两边都 > 20% 且差距 < 20%。 */
const BALANCED_MIN_SHARE = 0.2;
const BALANCED_GAP = 0.2;

interface CategoryProfile {
  /** 占比最大的分类（如果有）— 不过滤 'other'，但调用方可以按 id 判断。 */
  primary: QuickUsageEntry | null;
  /** 第二大分类 */
  secondary: QuickUsageEntry | null;
  /** 工作类总分钟数（code+browse+talk+design） */
  workMin: number;
  /** 娱乐类总分钟数（fun） */
  funMin: number;
  /** 工作类占比 */
  workShare: number;
  /** 娱乐类占比 */
  funShare: number;
  /** 顶级 fun 分类下用户具体在用什么 apps */
  funApps: QuickUsageEntry[];
}

function categoryProfile(
  categories: QuickUsageEntry[],
  topApps: QuickUsageEntry[],
  total: number,
): CategoryProfile {
  let workMin = 0;
  let funMin = 0;
  for (const c of categories) {
    if (WORK_LIKE_CATS.has(c.key)) workMin += c.minutes;
    if (FUN_LIKE_CATS.has(c.key)) funMin += c.minutes;
  }
  return {
    primary: categories[0] ?? null,
    secondary: categories[1] ?? null,
    workMin,
    funMin,
    workShare: total === 0 ? 0 : workMin / total,
    funShare: total === 0 ? 0 : funMin / total,
    funApps: topApps.filter((a) => FUN_LIKE_CATS.has(a.categoryId)),
  };
}

/** primary 是不是真正"显著"的主导分类——占比够高且不是 'other'。 */
function hasPrimary(profile: CategoryProfile): boolean {
  return (
    profile.primary != null &&
    profile.primary.key !== "other" &&
    profile.primary.percent >= PRIMARY_CATEGORY_THRESHOLD
  );
}

/** 字符串 hash（djb2 变种）。同一日期 + 段名永远拿到稳定的 idx，避免刷新页面文案抖动。 */
function hashString(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) {
    h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  }
  return Math.abs(h);
}

/** 在多个变体中按 seed 稳定挑一个。
 *  variantCount = 1 时：baseKey 是叶子字符串，直接 t(baseKey)。
 *  variantCount >= 2 时：baseKey 是嵌套对象 { v1, v2, ... }，按 hash 选一个走 t(baseKey.vN)。
 *  i18next 按 `.` 拆嵌套，所以变体必须以嵌套对象形式存在，不能写成 flat 兄弟 key。 */
function pickVariant(
  t: T,
  baseKey: string,
  variantCount: number,
  seed: string,
  params?: Record<string, unknown>,
): string {
  if (variantCount <= 1) return t(baseKey, params);
  const idx = hashString(seed + ":" + baseKey) % variantCount;
  return t(`${baseKey}.v${idx + 1}`, params);
}

// ───────────────────────── 日报：时间节奏 / 分类构成 ─────────────────────────

/** 日报时间节奏：基于 dayParts (night/morning/afternoon/evening) 分布选分支。 */
function dayTimePattern(data: QuickDaySummary, t: T, seed: string): string {
  const total = data.totalMinutes;
  const get = (k: string) => data.dayParts.find((p) => p.key === k)?.minutes ?? 0;
  const night = get("night");
  const morning = get("morning");
  const afternoon = get("afternoon");
  const evening = get("evening");
  const daytime = morning + afternoon;
  const nightShare = night / total;
  const eveningShare = evening / total;
  const daytimeShare = daytime / total;

  const peakLabel = data.peakHour != null ? `${String(data.peakHour).padStart(2, "0")}:00` : "—";

  // 凌晨占比偏高直接报警：跨午夜 = 健康信号最强
  if (nightShare >= 0.3 && night >= 60) {
    return pickVariant(t, "aiSummary.quick.analysis.day.timePattern.lateNight", 2, seed, {
      nightMinutes: formatMinutes(t, night),
      nightPct: formatPercent(nightShare),
    });
  }
  if (eveningShare >= 0.6) {
    return pickVariant(t, "aiSummary.quick.analysis.day.timePattern.eveningHeavy", 2, seed, {
      eveningMinutes: formatMinutes(t, evening),
      eveningPct: formatPercent(eveningShare),
      peak: peakLabel,
    });
  }
  if (daytimeShare >= 0.6 && morning >= afternoon) {
    return pickVariant(t, "aiSummary.quick.analysis.day.timePattern.morningHeavy", 2, seed, {
      morningMinutes: formatMinutes(t, morning),
      afternoonMinutes: formatMinutes(t, afternoon),
      peak: peakLabel,
    });
  }
  if (daytimeShare >= 0.6) {
    return pickVariant(t, "aiSummary.quick.analysis.day.timePattern.afternoonHeavy", 2, seed, {
      afternoonMinutes: formatMinutes(t, afternoon),
      morningMinutes: formatMinutes(t, morning),
      peak: peakLabel,
    });
  }
  if (data.activeHours <= 3) {
    return pickVariant(t, "aiSummary.quick.analysis.day.timePattern.sparse", 2, seed, {
      activeHours: data.activeHours,
      peak: peakLabel,
      peakMinutes: formatMinutes(t, data.peakHourMinutes),
    });
  }
  return pickVariant(t, "aiSummary.quick.analysis.day.timePattern.balanced", 2, seed, {
    activeHours: data.activeHours,
    peak: peakLabel,
  });
}

/** 分类构成段：列出主要分类各自的时长 + 主力应用。 */
function dayCategoryBreakdown(
  data: QuickDaySummary,
  t: T,
  catName: CatName,
  seed: string,
): string | null {
  // 占比低于 5% 的分类不算"主要"，避免噪声
  const visible = data.categories.filter((c) => c.percent >= 0.05).slice(0, 3);
  if (visible.length === 0) return null;

  const subItems = visible.map((c) => ({
    cat: c,
    apps: data.topApps.filter((a) => a.categoryId === c.key).slice(0, 2),
  }));

  if (subItems.length === 1) {
    const it = subItems[0];
    return pickVariant(
      t,
      "aiSummary.quick.analysis.day.categoryBreakdown.singleDominant",
      2,
      seed,
      {
        cat: catName(it.cat.key),
        minutes: formatMinutes(t, it.cat.minutes),
        pct: formatPercent(it.cat.percent),
        apps: it.apps.length > 0 ? formatAppList(it.apps, t) : "",
      },
    );
  }
  if (subItems.length === 2) {
    const [a, b] = subItems;
    return pickVariant(t, "aiSummary.quick.analysis.day.categoryBreakdown.twoCategories", 2, seed, {
      cat1: catName(a.cat.key),
      min1: formatMinutes(t, a.cat.minutes),
      pct1: formatPercent(a.cat.percent),
      apps1: a.apps.length > 0 ? formatAppList(a.apps, t) : "",
      cat2: catName(b.cat.key),
      min2: formatMinutes(t, b.cat.minutes),
      pct2: formatPercent(b.cat.percent),
      apps2: b.apps.length > 0 ? formatAppList(b.apps, t) : "",
    });
  }
  const [a, b, c] = subItems;
  return pickVariant(t, "aiSummary.quick.analysis.day.categoryBreakdown.threeCategories", 2, seed, {
    cat1: catName(a.cat.key),
    min1: formatMinutes(t, a.cat.minutes),
    pct1: formatPercent(a.cat.percent),
    apps1: a.apps.length > 0 ? formatAppList(a.apps, t) : "",
    cat2: catName(b.cat.key),
    min2: formatMinutes(t, b.cat.minutes),
    pct2: formatPercent(b.cat.percent),
    cat3: catName(c.cat.key),
    min3: formatMinutes(t, c.cat.minutes),
    pct3: formatPercent(c.cat.percent),
  });
}

/** 日报亮点：当期内的"突出事实"组合 1-2 句。不依赖历史基线，只用数据自身的对比。 */
function dayHighlights(data: QuickDaySummary, t: T, seed: string): string | null {
  const total = data.totalMinutes;
  const facts: string[] = [];

  // —— Top 1 vs Top 2 差距大：主线感很强
  const top1 = data.topApps[0];
  const top2 = data.topApps[1];
  if (top1 && top2 && top1.minutes >= top2.minutes * 2 && top1.minutes >= 30) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.day.highlights.topGap", 2, seed, {
        top1: top1.key,
        top1Min: formatMinutes(t, top1.minutes),
        top2: top2.key,
        top2Min: formatMinutes(t, top2.minutes),
      }),
    );
  }

  // —— 单一应用占比超高
  if (top1 && top1.percent >= 0.5 && top1.minutes >= 60) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.day.highlights.topShareHigh", 2, seed, {
        app: top1.key,
        pct: formatPercent(top1.percent),
      }),
    );
  }

  // —— peak hour 单独承担了相当占比
  if (
    data.peakHour != null &&
    total > 0 &&
    data.peakHourMinutes / total >= 0.25 &&
    data.peakHourMinutes >= 30
  ) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.day.highlights.peakHourHigh", 2, seed, {
        hour: String(data.peakHour).padStart(2, "0"),
        nextHour: String(data.peakHour + 1).padStart(2, "0"),
        peakMinutes: formatMinutes(t, data.peakHourMinutes),
        peakPct: formatPercent(data.peakHourMinutes / total),
      }),
    );
  }

  // —— 凌晨段值得点名
  const nightPart = data.dayParts.find((p) => p.key === "night");
  if (nightPart && nightPart.minutes >= 30) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.day.highlights.nightPresence", 2, seed, {
        nightMinutes: formatMinutes(t, nightPart.minutes),
      }),
    );
  }

  // —— Top 应用占比低 + 分类数多：今日"广而散"
  if (top1 && top1.percent < 0.18 && data.categories.length >= 3) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.day.highlights.broadSpread", 2, seed, {
        topPct: formatPercent(top1.percent),
        categoryCount: data.categories.length,
      }),
    );
  }

  if (facts.length === 0) {
    return t("aiSummary.quick.analysis.day.highlights.none");
  }
  // 最多 2 条事实拼接；用 i18n 的句子拼接符（中英日各有空格/无空格规则）
  return facts.slice(0, 2).join(t("aiSummary.quick.analysis.factsJoin"));
}

/** 日报专注度：综合 top app 占比 + peak hour 占比 + 分类数。 */
function dayFocus(data: QuickDaySummary, t: T, seed: string): string {
  const top1 = data.topApps[0];
  const topShare = top1?.percent ?? 0;
  const peakShare = data.totalMinutes > 0 ? data.peakHourMinutes / data.totalMinutes : 0;
  const visibleCats = data.categories.filter((c) => c.percent >= 0.05).length;
  const topApp = top1?.key ?? "";

  // 极少数据
  if (data.totalMinutes < LIGHT_DAY_MINUTES) {
    return pickVariant(t, "aiSummary.quick.analysis.day.focus.tooShort", 2, seed, {
      topPct: formatPercent(topShare),
      categoryCount: visibleCats,
    });
  }
  if (topShare >= 0.4 && visibleCats <= 2) {
    return pickVariant(t, "aiSummary.quick.analysis.day.focus.highFocus", 2, seed, {
      app: topApp,
      topPct: formatPercent(topShare),
      peakPct: formatPercent(peakShare),
      categoryCount: visibleCats,
    });
  }
  if (topShare >= 0.25) {
    return pickVariant(t, "aiSummary.quick.analysis.day.focus.goodFocus", 2, seed, {
      app: topApp,
      topPct: formatPercent(topShare),
      peakPct: formatPercent(peakShare),
      categoryCount: visibleCats,
    });
  }
  if (topShare >= 0.15 && visibleCats <= 3) {
    return pickVariant(t, "aiSummary.quick.analysis.day.focus.moderate", 2, seed, {
      topPct: formatPercent(topShare),
      peakPct: formatPercent(peakShare),
      categoryCount: visibleCats,
    });
  }
  return pickVariant(t, "aiSummary.quick.analysis.day.focus.fragmented", 2, seed, {
    topPct: formatPercent(topShare),
    categoryCount: visibleCats,
  });
}

// ───────────────────────── 日报 ─────────────────────────

/** 日报维度的分析。 */
export function buildDayAnalysis(data: QuickDaySummary, t: T, catName: CatName): QuickAnalysis {
  if (data.totalMinutes === 0) {
    return {
      workOverview: null,
      timePattern: null,
      categoryBreakdown: null,
      highlights: null,
      focus: null,
      efficiency: null,
      suggestion: null,
      summary: t("aiSummary.quick.analysis.day.summary.minimal"),
    };
  }
  const seed = `day:${data.date}`;

  const profile = categoryProfile(data.categories, data.topApps, data.totalMinutes);

  // —— 工作内容概述 ——
  let workOverview: string;
  if (hasPrimary(profile) && data.topApps.length >= 1) {
    workOverview = t("aiSummary.quick.analysis.day.workOverview.categoryLed", {
      category: catName(profile.primary!.key),
      pct: formatPercent(profile.primary!.percent),
      apps: formatAppList(data.topApps.slice(0, 3), t),
    });
  } else if (data.topApps.length === 1) {
    workOverview = t("aiSummary.quick.analysis.day.workOverview.single", {
      topApp: formatAppEntry(data.topApps[0], t),
    });
  } else if (data.topApps.length === 0) {
    workOverview = t("aiSummary.quick.analysis.day.workOverview.empty");
  } else {
    workOverview = t("aiSummary.quick.analysis.day.workOverview.normal", {
      apps: formatAppList(data.topApps, t),
    });
  }

  // —— 效率评估 ——
  const topShare = data.topApps[0]?.percent ?? 0;
  let efficiency: string;
  if (data.totalMinutes < LIGHT_DAY_MINUTES) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.day.efficiency.light", 2, seed);
  } else if (profile.funShare >= FUN_HEAVY_THRESHOLD) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.day.efficiency.funHeavy", 2, seed, {
      pct: formatPercent(profile.funShare),
      apps:
        profile.funApps.length > 0
          ? formatAppList(profile.funApps.slice(0, 2), t)
          : t("aiSummary.quick.analysis.day.efficiency.funAppsFallback"),
    });
  } else if (
    profile.workShare >= BALANCED_MIN_SHARE &&
    profile.funShare >= BALANCED_MIN_SHARE &&
    Math.abs(profile.workShare - profile.funShare) <= BALANCED_GAP
  ) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.day.efficiency.balanced", 2, seed, {
      workPct: formatPercent(profile.workShare),
      funPct: formatPercent(profile.funShare),
    });
  } else if (topShare >= 0.4) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.day.efficiency.focused", 2, seed, {
      pct: formatPercent(topShare),
      app: data.topApps[0]?.key ?? "",
    });
  } else if (topShare >= 0.2) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.day.efficiency.steady", 3, seed);
  } else {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.day.efficiency.scattered", 2, seed);
  }

  // —— 改进建议 ——
  const nightPart = data.dayParts.find((p) => p.key === "night");
  const eveningPart = data.dayParts.find((p) => p.key === "evening");
  const morningPart = data.dayParts.find((p) => p.key === "morning");
  const afternoonPart = data.dayParts.find((p) => p.key === "afternoon");
  const dayWorkMin = (morningPart?.minutes ?? 0) + (afternoonPart?.minutes ?? 0);
  const eveningMin = eveningPart?.minutes ?? 0;
  const nightMin = nightPart?.minutes ?? 0;

  let suggestion: string;
  if (nightMin >= NIGHT_HEAVY_MINUTES) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.day.suggestion.nightHeavy", 2, seed, {
      minutes: formatMinutes(t, nightMin),
    });
  } else if (profile.funShare >= FUN_HEAVY_THRESHOLD) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.day.suggestion.funHeavy", 2, seed, {
      pct: formatPercent(profile.funShare),
    });
  } else if (eveningMin > dayWorkMin && eveningMin >= 60) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.day.suggestion.eveningHeavy", 2, seed);
  } else if (topShare < 0.2 && data.topApps.length >= 4) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.day.suggestion.scattered", 2, seed);
  } else if (data.activeHours <= 3 && data.totalMinutes >= LIGHT_DAY_MINUTES) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.day.suggestion.short", 2, seed);
  } else if (profile.workShare >= 0.5 && data.totalMinutes >= HEAVY_DAY_MINUTES) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.day.suggestion.workIntense", 2, seed);
  } else {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.day.suggestion.default", 3, seed);
  }

  // —— 今日小结 ——
  let summary: string;
  if (data.totalMinutes >= HEAVY_DAY_MINUTES) {
    summary =
      profile.workShare >= 0.5
        ? pickVariant(t, "aiSummary.quick.analysis.day.summary.heavyWork", 2, seed)
        : pickVariant(t, "aiSummary.quick.analysis.day.summary.heavy", 2, seed);
  } else if (data.totalMinutes >= BALANCED_DAY_MINUTES) {
    summary = hasPrimary(profile)
      ? pickVariant(t, "aiSummary.quick.analysis.day.summary.balancedWithCategory", 2, seed, {
          category: catName(profile.primary!.key),
        })
      : pickVariant(t, "aiSummary.quick.analysis.day.summary.balanced", 3, seed);
  } else if (data.totalMinutes >= LIGHT_DAY_MINUTES) {
    summary =
      profile.funShare >= 0.5
        ? pickVariant(t, "aiSummary.quick.analysis.day.summary.lightFun", 2, seed)
        : pickVariant(t, "aiSummary.quick.analysis.day.summary.light", 2, seed);
  } else {
    summary = t("aiSummary.quick.analysis.day.summary.minimal");
  }

  const timePattern = dayTimePattern(data, t, seed);
  const categoryBreakdown = dayCategoryBreakdown(data, t, catName, seed);
  const highlights = dayHighlights(data, t, seed);
  const focus = dayFocus(data, t, seed);

  return {
    workOverview,
    timePattern,
    categoryBreakdown,
    highlights,
    focus,
    efficiency,
    suggestion,
    summary,
  };
}

// ───────────────────────── 周报：节奏分布 / 分类构成 ─────────────────────────

const HEAVY_WEEK_MINUTES = HEAVY_DAY_MINUTES * 5; // 一周 > 30h
const BALANCED_WEEK_MINUTES = BALANCED_DAY_MINUTES * 4; // 一周 > 12h
const LIGHT_WEEK_MINUTES = LIGHT_DAY_MINUTES * 4; // 一周 > 4h

const WEEKDAY_KEYS = ["0", "1", "2", "3", "4", "5", "6"]; // 周一..周日

/** 周报节奏：工作日 vs 周末分布 + 高峰日。 */
function weekTimePattern(data: QuickWeekSummary, t: T, seed: string): string {
  const series = data.dailySeries;
  if (series.length === 0) {
    return pickVariant(t, "aiSummary.quick.analysis.week.timePattern.balanced", 2, seed, {
      activeDays: data.activeDays,
    });
  }
  // dailySeries 长度可能不是 7（API 给的是周一到周日，但兼容性兜底）
  const week = series.slice(0, 7);
  const weekdayMin = week.slice(0, 5).reduce((s, p) => s + p.minutes, 0);
  const weekendMin = week.slice(5).reduce((s, p) => s + p.minutes, 0);
  const total = weekdayMin + weekendMin;
  if (total === 0) {
    return pickVariant(t, "aiSummary.quick.analysis.week.timePattern.balanced", 2, seed, {
      activeDays: data.activeDays,
    });
  }
  const weekdayShare = weekdayMin / total;
  const volatility = computeVolatility(week);

  // 找峰值日 + 谷值日（按 weekday 0..6，对应周一..周日）
  let peakIdx = 0;
  let lowIdx = -1;
  let lowMin = Number.POSITIVE_INFINITY;
  for (let i = 0; i < week.length; i++) {
    if (week[i].minutes > week[peakIdx].minutes) peakIdx = i;
    if (week[i].minutes > 0 && week[i].minutes < lowMin) {
      lowMin = week[i].minutes;
      lowIdx = i;
    }
  }
  const peakKey = t(`aiSummary.quick.weekday.${WEEKDAY_KEYS[peakIdx] ?? "0"}`);
  const peakMin = formatMinutes(t, week[peakIdx]?.minutes ?? 0);

  if (weekdayShare >= 0.85) {
    return pickVariant(t, "aiSummary.quick.analysis.week.timePattern.weekdayDominant", 2, seed, {
      pct: formatPercent(weekdayShare),
      peak: peakKey,
      peakMinutes: peakMin,
    });
  }
  if (weekdayShare <= 0.4) {
    return pickVariant(t, "aiSummary.quick.analysis.week.timePattern.weekendDominant", 2, seed, {
      pct: formatPercent(1 - weekdayShare),
      peak: peakKey,
      peakMinutes: peakMin,
    });
  }
  if (volatility >= 2.5) {
    const lowKey = lowIdx >= 0 ? t(`aiSummary.quick.weekday.${WEEKDAY_KEYS[lowIdx]}`) : peakKey;
    const lowMinFmt = lowIdx >= 0 ? formatMinutes(t, week[lowIdx].minutes) : "—";
    return pickVariant(t, "aiSummary.quick.analysis.week.timePattern.volatile", 2, seed, {
      peak: peakKey,
      peakMinutes: peakMin,
      low: lowKey,
      lowMinutes: lowMinFmt,
    });
  }
  return pickVariant(t, "aiSummary.quick.analysis.week.timePattern.consistent", 2, seed, {
    activeDays: data.activeDays,
    peak: peakKey,
    peakMinutes: peakMin,
  });
}

/** 周报分类构成：同 day，但门槛和文案略改。 */
function weekCategoryBreakdown(
  data: QuickWeekSummary,
  t: T,
  catName: CatName,
  seed: string,
): string | null {
  const visible = data.categories.filter((c) => c.percent >= 0.05).slice(0, 3);
  if (visible.length === 0) return null;
  const subItems = visible.map((c) => ({
    cat: c,
    apps: data.topApps.filter((a) => a.categoryId === c.key).slice(0, 2),
  }));
  if (subItems.length === 1) {
    const it = subItems[0];
    return pickVariant(
      t,
      "aiSummary.quick.analysis.week.categoryBreakdown.singleDominant",
      2,
      seed,
      {
        cat: catName(it.cat.key),
        minutes: formatMinutes(t, it.cat.minutes),
        pct: formatPercent(it.cat.percent),
        apps: it.apps.length > 0 ? formatAppList(it.apps, t) : "",
      },
    );
  }
  if (subItems.length === 2) {
    const [a, b] = subItems;
    return pickVariant(
      t,
      "aiSummary.quick.analysis.week.categoryBreakdown.twoCategories",
      2,
      seed,
      {
        cat1: catName(a.cat.key),
        min1: formatMinutes(t, a.cat.minutes),
        pct1: formatPercent(a.cat.percent),
        apps1: a.apps.length > 0 ? formatAppList(a.apps, t) : "",
        cat2: catName(b.cat.key),
        min2: formatMinutes(t, b.cat.minutes),
        pct2: formatPercent(b.cat.percent),
        apps2: b.apps.length > 0 ? formatAppList(b.apps, t) : "",
      },
    );
  }
  const [a, b, c] = subItems;
  return pickVariant(
    t,
    "aiSummary.quick.analysis.week.categoryBreakdown.threeCategories",
    2,
    seed,
    {
      cat1: catName(a.cat.key),
      min1: formatMinutes(t, a.cat.minutes),
      pct1: formatPercent(a.cat.percent),
      apps1: a.apps.length > 0 ? formatAppList(a.apps, t) : "",
      cat2: catName(b.cat.key),
      min2: formatMinutes(t, b.cat.minutes),
      pct2: formatPercent(b.cat.percent),
      cat3: catName(c.cat.key),
      min3: formatMinutes(t, c.cat.minutes),
      pct3: formatPercent(c.cat.percent),
    },
  );
}

/** 周报亮点：工作日 vs 周末、peak day 突出、top app 突出。 */
function weekHighlights(data: QuickWeekSummary, t: T, seed: string): string | null {
  const total = data.totalMinutes;
  const facts: string[] = [];

  // peak day 单独占整周相当比重
  const series = data.dailySeries.slice(0, 7);
  let peakIdx = 0;
  for (let i = 0; i < series.length; i++) {
    if (series[i].minutes > series[peakIdx].minutes) peakIdx = i;
  }
  const peakMin = series[peakIdx]?.minutes ?? 0;
  if (total > 0 && peakMin / total >= 0.25 && peakMin >= 60) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.week.highlights.peakDayHigh", 2, seed, {
        weekday: t(`aiSummary.quick.weekday.${peakIdx}`),
        peakMinutes: formatMinutes(t, peakMin),
        peakPct: formatPercent(peakMin / total),
      }),
    );
  }

  // 工作日 vs 周末显著不平衡
  const weekdayMin = series.slice(0, 5).reduce((s, p) => s + p.minutes, 0);
  const weekendMin = series.slice(5).reduce((s, p) => s + p.minutes, 0);
  if (total > 0) {
    const weekdayShare = weekdayMin / total;
    if (weekdayShare >= 0.9 && weekendMin === 0) {
      facts.push(
        pickVariant(t, "aiSummary.quick.analysis.week.highlights.weekendOff", 2, seed, {
          weekdayMinutes: formatMinutes(t, weekdayMin),
        }),
      );
    } else if (weekdayShare <= 0.3 && weekendMin > 0) {
      facts.push(
        pickVariant(t, "aiSummary.quick.analysis.week.highlights.weekendMain", 2, seed, {
          weekendMinutes: formatMinutes(t, weekendMin),
          weekendPct: formatPercent(weekendMin / total),
        }),
      );
    }
  }

  // top 应用占比超高
  const top1 = data.topApps[0];
  if (top1 && top1.percent >= 0.4) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.week.highlights.topShareHigh", 2, seed, {
        app: top1.key,
        pct: formatPercent(top1.percent),
      }),
    );
  }

  // 覆盖率（活跃日数）= 7：全勤
  if (data.activeDays === 7) {
    facts.push(pickVariant(t, "aiSummary.quick.analysis.week.highlights.fullAttendance", 2, seed));
  }

  if (facts.length === 0) {
    return t("aiSummary.quick.analysis.week.highlights.none");
  }
  return facts.slice(0, 2).join(t("aiSummary.quick.analysis.factsJoin"));
}

/** 周报专注度。 */
function weekFocus(data: QuickWeekSummary, t: T, seed: string): string {
  const top1 = data.topApps[0];
  const topShare = top1?.percent ?? 0;
  const visibleCats = data.categories.filter((c) => c.percent >= 0.05).length;
  const topApp = top1?.key ?? "";

  if (data.totalMinutes < LIGHT_WEEK_MINUTES) {
    return pickVariant(t, "aiSummary.quick.analysis.week.focus.tooShort", 2, seed, {
      topPct: formatPercent(topShare),
      categoryCount: visibleCats,
    });
  }
  if (topShare >= 0.3 && visibleCats <= 2) {
    return pickVariant(t, "aiSummary.quick.analysis.week.focus.highFocus", 2, seed, {
      app: topApp,
      topPct: formatPercent(topShare),
      activeDays: data.activeDays,
      categoryCount: visibleCats,
    });
  }
  if (topShare >= 0.2) {
    return pickVariant(t, "aiSummary.quick.analysis.week.focus.goodFocus", 2, seed, {
      app: topApp,
      topPct: formatPercent(topShare),
      activeDays: data.activeDays,
      categoryCount: visibleCats,
    });
  }
  if (topShare >= 0.1 && visibleCats <= 4) {
    return pickVariant(t, "aiSummary.quick.analysis.week.focus.moderate", 2, seed, {
      topPct: formatPercent(topShare),
      categoryCount: visibleCats,
    });
  }
  return pickVariant(t, "aiSummary.quick.analysis.week.focus.fragmented", 2, seed, {
    topPct: formatPercent(topShare),
    categoryCount: visibleCats,
  });
}

// ───────────────────────── 周报 ─────────────────────────

/** 周报维度的分析。 */
export function buildWeekAnalysis(data: QuickWeekSummary, t: T, catName: CatName): QuickAnalysis {
  if (data.totalMinutes === 0) {
    return {
      workOverview: null,
      timePattern: null,
      categoryBreakdown: null,
      highlights: null,
      focus: null,
      efficiency: null,
      suggestion: null,
      summary: t("aiSummary.quick.analysis.week.summary.minimal"),
    };
  }
  const seed = `week:${data.weekStart}`;

  const profile = categoryProfile(data.categories, data.topApps, data.totalMinutes);

  // —— 工作内容概述 ——
  const workOverview = hasPrimary(profile)
    ? t("aiSummary.quick.analysis.week.workOverview.categoryLed", {
        category: catName(profile.primary!.key),
        pct: formatPercent(profile.primary!.percent),
        apps: formatAppList(data.topApps.slice(0, 3), t),
        activeDays: data.activeDays,
      })
    : t("aiSummary.quick.analysis.week.workOverview.normal", {
        apps: formatAppList(data.topApps, t),
        activeDays: data.activeDays,
      });

  // —— 效率评估 ——
  const topShare = data.topApps[0]?.percent ?? 0;
  const volatility = computeVolatility(data.dailySeries);
  let efficiency: string;
  if (data.totalMinutes < LIGHT_WEEK_MINUTES) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.week.efficiency.light", 2, seed);
  } else if (profile.funShare >= FUN_HEAVY_THRESHOLD) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.week.efficiency.funHeavy", 2, seed, {
      pct: formatPercent(profile.funShare),
    });
  } else if (
    profile.workShare >= BALANCED_MIN_SHARE &&
    profile.funShare >= BALANCED_MIN_SHARE &&
    Math.abs(profile.workShare - profile.funShare) <= BALANCED_GAP
  ) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.week.efficiency.balanced", 2, seed, {
      workPct: formatPercent(profile.workShare),
      funPct: formatPercent(profile.funShare),
    });
  } else if (topShare >= 0.3) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.week.efficiency.focused", 2, seed, {
      pct: formatPercent(topShare),
      app: data.topApps[0]?.key ?? "",
    });
  } else if (volatility > 2.5) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.week.efficiency.volatile", 2, seed);
  } else if (topShare >= 0.15) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.week.efficiency.steady", 3, seed);
  } else {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.week.efficiency.scattered", 2, seed);
  }

  // —— 改进建议 ——
  let suggestion: string;
  if (data.activeDays <= 3) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.week.suggestion.sparse", 2, seed, {
      activeDays: data.activeDays,
    });
  } else if (profile.funShare >= FUN_HEAVY_THRESHOLD) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.week.suggestion.funHeavy", 2, seed, {
      pct: formatPercent(profile.funShare),
    });
  } else if (volatility > 2.5) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.week.suggestion.volatile", 2, seed);
  } else if (topShare < 0.15) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.week.suggestion.scattered", 2, seed);
  } else if (data.activeDays >= 6 && profile.workShare >= 0.5) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.week.suggestion.solidPace", 2, seed);
  } else {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.week.suggestion.default", 3, seed);
  }

  // —— 本周小结 ——
  let summary: string;
  if (data.totalMinutes >= HEAVY_WEEK_MINUTES) {
    summary = hasPrimary(profile)
      ? pickVariant(t, "aiSummary.quick.analysis.week.summary.heavyWithCategory", 2, seed, {
          category: catName(profile.primary!.key),
        })
      : pickVariant(t, "aiSummary.quick.analysis.week.summary.heavy", 2, seed);
  } else if (data.totalMinutes >= BALANCED_WEEK_MINUTES) {
    summary = pickVariant(t, "aiSummary.quick.analysis.week.summary.balanced", 3, seed);
  } else if (data.totalMinutes >= LIGHT_WEEK_MINUTES) {
    summary =
      profile.funShare >= 0.5
        ? pickVariant(t, "aiSummary.quick.analysis.week.summary.lightFun", 2, seed)
        : pickVariant(t, "aiSummary.quick.analysis.week.summary.light", 2, seed);
  } else {
    summary = t("aiSummary.quick.analysis.week.summary.minimal");
  }

  const timePattern = weekTimePattern(data, t, seed);
  const categoryBreakdown = weekCategoryBreakdown(data, t, catName, seed);
  const highlights = weekHighlights(data, t, seed);
  const focus = weekFocus(data, t, seed);

  return {
    workOverview,
    timePattern,
    categoryBreakdown,
    highlights,
    focus,
    efficiency,
    suggestion,
    summary,
  };
}

// ───────────────────────── 月报：节奏分布 / 分类构成 ─────────────────────────

/** 月报节奏：上半月 vs 下半月 + 高峰/谷值日。 */
function monthTimePattern(data: QuickMonthSummary, t: T, seed: string): string {
  const series = data.dailySeries;
  if (series.length === 0) {
    return pickVariant(t, "aiSummary.quick.analysis.month.timePattern.consistent", 2, seed, {
      activeDays: data.activeDays,
    });
  }
  const mid = Math.ceil(series.length / 2);
  const firstHalf = series.slice(0, mid).reduce((s, p) => s + p.minutes, 0);
  const secondHalf = series.slice(mid).reduce((s, p) => s + p.minutes, 0);
  const total = firstHalf + secondHalf;
  const volatility = computeVolatility(series);

  // 找峰值日 + 谷值日
  let peakIdx = 0;
  let lowIdx = -1;
  let lowMin = Number.POSITIVE_INFINITY;
  for (let i = 0; i < series.length; i++) {
    if (series[i].minutes > series[peakIdx].minutes) peakIdx = i;
    if (series[i].minutes > 0 && series[i].minutes < lowMin) {
      lowMin = series[i].minutes;
      lowIdx = i;
    }
  }
  const peakDate = series[peakIdx]?.date ?? "—";
  const peakMin = formatMinutes(t, series[peakIdx]?.minutes ?? 0);

  if (total === 0) {
    return pickVariant(t, "aiSummary.quick.analysis.month.timePattern.consistent", 2, seed, {
      activeDays: data.activeDays,
    });
  }
  const firstShare = firstHalf / total;

  if (firstShare >= 0.7) {
    return pickVariant(t, "aiSummary.quick.analysis.month.timePattern.firstHalfHeavy", 2, seed, {
      pct: formatPercent(firstShare),
      peakDate,
      peakMinutes: peakMin,
    });
  }
  if (firstShare <= 0.3) {
    return pickVariant(t, "aiSummary.quick.analysis.month.timePattern.secondHalfHeavy", 2, seed, {
      pct: formatPercent(1 - firstShare),
      peakDate,
      peakMinutes: peakMin,
    });
  }
  if (volatility >= 3.0) {
    const lowDate = lowIdx >= 0 ? series[lowIdx].date : peakDate;
    const lowMinFmt = lowIdx >= 0 ? formatMinutes(t, series[lowIdx].minutes) : "—";
    return pickVariant(t, "aiSummary.quick.analysis.month.timePattern.volatile", 2, seed, {
      peakDate,
      peakMinutes: peakMin,
      lowDate,
      lowMinutes: lowMinFmt,
    });
  }
  return pickVariant(t, "aiSummary.quick.analysis.month.timePattern.consistent", 2, seed, {
    activeDays: data.activeDays,
    peakDate,
    peakMinutes: peakMin,
  });
}

/** 月报分类构成。 */
function monthCategoryBreakdown(
  data: QuickMonthSummary,
  t: T,
  catName: CatName,
  seed: string,
): string | null {
  const visible = data.categories.filter((c) => c.percent >= 0.05).slice(0, 3);
  if (visible.length === 0) return null;
  const subItems = visible.map((c) => ({
    cat: c,
    apps: data.topApps.filter((a) => a.categoryId === c.key).slice(0, 2),
  }));
  if (subItems.length === 1) {
    const it = subItems[0];
    return pickVariant(
      t,
      "aiSummary.quick.analysis.month.categoryBreakdown.singleDominant",
      2,
      seed,
      {
        cat: catName(it.cat.key),
        minutes: formatMinutes(t, it.cat.minutes),
        pct: formatPercent(it.cat.percent),
        apps: it.apps.length > 0 ? formatAppList(it.apps, t) : "",
      },
    );
  }
  if (subItems.length === 2) {
    const [a, b] = subItems;
    return pickVariant(
      t,
      "aiSummary.quick.analysis.month.categoryBreakdown.twoCategories",
      2,
      seed,
      {
        cat1: catName(a.cat.key),
        min1: formatMinutes(t, a.cat.minutes),
        pct1: formatPercent(a.cat.percent),
        apps1: a.apps.length > 0 ? formatAppList(a.apps, t) : "",
        cat2: catName(b.cat.key),
        min2: formatMinutes(t, b.cat.minutes),
        pct2: formatPercent(b.cat.percent),
        apps2: b.apps.length > 0 ? formatAppList(b.apps, t) : "",
      },
    );
  }
  const [a, b, c] = subItems;
  return pickVariant(
    t,
    "aiSummary.quick.analysis.month.categoryBreakdown.threeCategories",
    2,
    seed,
    {
      cat1: catName(a.cat.key),
      min1: formatMinutes(t, a.cat.minutes),
      pct1: formatPercent(a.cat.percent),
      apps1: a.apps.length > 0 ? formatAppList(a.apps, t) : "",
      cat2: catName(b.cat.key),
      min2: formatMinutes(t, b.cat.minutes),
      pct2: formatPercent(b.cat.percent),
      cat3: catName(c.cat.key),
      min3: formatMinutes(t, c.cat.minutes),
      pct3: formatPercent(c.cat.percent),
    },
  );
}

/** 月报亮点。 */
function monthHighlights(data: QuickMonthSummary, t: T, seed: string): string | null {
  const total = data.totalMinutes;
  const facts: string[] = [];

  // peak day 占整月较高比例
  const series = data.dailySeries;
  let peakIdx = 0;
  for (let i = 0; i < series.length; i++) {
    if (series[i].minutes > series[peakIdx].minutes) peakIdx = i;
  }
  const peakDate = series[peakIdx]?.date ?? "—";
  const peakMin = series[peakIdx]?.minutes ?? 0;
  if (total > 0 && peakMin / total >= 0.12 && peakMin >= 60) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.month.highlights.peakDayHigh", 2, seed, {
        peakDate,
        peakMinutes: formatMinutes(t, peakMin),
        peakPct: formatPercent(peakMin / total),
      }),
    );
  }

  // 全月覆盖率高（连续度好）
  const coverage = data.activeDays / Math.max(1, data.totalDays);
  if (coverage >= 0.9) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.month.highlights.fullCoverage", 2, seed, {
        activeDays: data.activeDays,
        totalDays: data.totalDays,
      }),
    );
  }

  // top 应用占比超高
  const top1 = data.topApps[0];
  if (top1 && top1.percent >= 0.4) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.month.highlights.topShareHigh", 2, seed, {
        app: top1.key,
        pct: formatPercent(top1.percent),
      }),
    );
  }

  // 跨度很广（活跃日多但分类也多 = 涉猎广）
  if (data.activeDays >= 20 && data.categories.length >= 4) {
    facts.push(
      pickVariant(t, "aiSummary.quick.analysis.month.highlights.broadSpread", 2, seed, {
        activeDays: data.activeDays,
        categoryCount: data.categories.length,
      }),
    );
  }

  if (facts.length === 0) {
    return t("aiSummary.quick.analysis.month.highlights.none");
  }
  return facts.slice(0, 2).join(t("aiSummary.quick.analysis.factsJoin"));
}

/** 月报专注度。 */
function monthFocus(data: QuickMonthSummary, t: T, seed: string): string {
  const top1 = data.topApps[0];
  const topShare = top1?.percent ?? 0;
  const coverage = data.activeDays / Math.max(1, data.totalDays);
  const visibleCats = data.categories.filter((c) => c.percent >= 0.05).length;
  const topApp = top1?.key ?? "";

  if (coverage < 0.3 || data.totalMinutes < HEAVY_DAY_MINUTES) {
    return pickVariant(t, "aiSummary.quick.analysis.month.focus.tooShort", 2, seed, {
      topPct: formatPercent(topShare),
      categoryCount: visibleCats,
    });
  }
  if (topShare >= 0.3 && visibleCats <= 2) {
    return pickVariant(t, "aiSummary.quick.analysis.month.focus.highFocus", 2, seed, {
      app: topApp,
      topPct: formatPercent(topShare),
      activeDays: data.activeDays,
      categoryCount: visibleCats,
    });
  }
  if (topShare >= 0.2) {
    return pickVariant(t, "aiSummary.quick.analysis.month.focus.goodFocus", 2, seed, {
      app: topApp,
      topPct: formatPercent(topShare),
      activeDays: data.activeDays,
      categoryCount: visibleCats,
    });
  }
  if (topShare >= 0.1 && visibleCats <= 4) {
    return pickVariant(t, "aiSummary.quick.analysis.month.focus.moderate", 2, seed, {
      topPct: formatPercent(topShare),
      categoryCount: visibleCats,
    });
  }
  return pickVariant(t, "aiSummary.quick.analysis.month.focus.fragmented", 2, seed, {
    topPct: formatPercent(topShare),
    categoryCount: visibleCats,
  });
}

// ───────────────────────── 月报 ─────────────────────────

/** 月报维度的分析。 */
export function buildMonthAnalysis(data: QuickMonthSummary, t: T, catName: CatName): QuickAnalysis {
  if (data.totalMinutes === 0) {
    return {
      workOverview: null,
      timePattern: null,
      categoryBreakdown: null,
      highlights: null,
      focus: null,
      efficiency: null,
      suggestion: null,
      summary: t("aiSummary.quick.analysis.month.summary.minimal"),
    };
  }
  const seed = `month:${data.monthStart}`;

  const profile = categoryProfile(data.categories, data.topApps, data.totalMinutes);

  // —— 工作内容概述 ——
  const workOverview = hasPrimary(profile)
    ? t("aiSummary.quick.analysis.month.workOverview.categoryLed", {
        category: catName(profile.primary!.key),
        pct: formatPercent(profile.primary!.percent),
        apps: formatAppList(data.topApps.slice(0, 3), t),
        activeDays: data.activeDays,
        totalDays: data.totalDays,
      })
    : t("aiSummary.quick.analysis.month.workOverview.normal", {
        apps: formatAppList(data.topApps, t),
        activeDays: data.activeDays,
        totalDays: data.totalDays,
      });

  // —— 效率评估 ——
  const topShare = data.topApps[0]?.percent ?? 0;
  const coverage = data.activeDays / Math.max(1, data.totalDays);
  const volatility = computeVolatility(data.dailySeries);
  let efficiency: string;
  if (coverage < 0.3) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.month.efficiency.sparse", 2, seed, {
      pct: formatPercent(coverage),
    });
  } else if (profile.funShare >= FUN_HEAVY_THRESHOLD) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.month.efficiency.funHeavy", 2, seed, {
      pct: formatPercent(profile.funShare),
    });
  } else if (
    profile.workShare >= BALANCED_MIN_SHARE &&
    profile.funShare >= BALANCED_MIN_SHARE &&
    Math.abs(profile.workShare - profile.funShare) <= BALANCED_GAP
  ) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.month.efficiency.balanced", 2, seed, {
      workPct: formatPercent(profile.workShare),
      funPct: formatPercent(profile.funShare),
    });
  } else if (topShare >= 0.3) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.month.efficiency.focused", 2, seed, {
      pct: formatPercent(topShare),
      app: data.topApps[0]?.key ?? "",
    });
  } else if (volatility > 3.0) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.month.efficiency.volatile", 2, seed);
  } else if (topShare >= 0.15) {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.month.efficiency.steady", 3, seed);
  } else {
    efficiency = pickVariant(t, "aiSummary.quick.analysis.month.efficiency.scattered", 2, seed);
  }

  // —— 改进建议 ——
  let suggestion: string;
  if (coverage < 0.3) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.month.suggestion.sparse", 2, seed);
  } else if (profile.funShare >= FUN_HEAVY_THRESHOLD) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.month.suggestion.funHeavy", 2, seed, {
      pct: formatPercent(profile.funShare),
    });
  } else if (volatility > 3.0) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.month.suggestion.volatile", 2, seed);
  } else if (topShare < 0.15 && data.topApps.length >= 4) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.month.suggestion.scattered", 2, seed);
  } else if (coverage >= 0.8 && profile.workShare >= 0.5) {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.month.suggestion.solidPace", 2, seed);
  } else {
    suggestion = pickVariant(t, "aiSummary.quick.analysis.month.suggestion.default", 3, seed);
  }

  // —— 月份小结 ——
  const expectedHeavy = HEAVY_DAY_MINUTES * data.totalDays * 0.7;
  const expectedBalanced = BALANCED_DAY_MINUTES * data.totalDays * 0.5;
  let summary: string;
  if (data.totalMinutes >= expectedHeavy) {
    summary = hasPrimary(profile)
      ? pickVariant(t, "aiSummary.quick.analysis.month.summary.heavyWithCategory", 2, seed, {
          category: catName(profile.primary!.key),
        })
      : pickVariant(t, "aiSummary.quick.analysis.month.summary.heavy", 2, seed);
  } else if (data.totalMinutes >= expectedBalanced) {
    summary = pickVariant(t, "aiSummary.quick.analysis.month.summary.balanced", 3, seed);
  } else if (data.totalMinutes >= LIGHT_DAY_MINUTES * data.totalDays * 0.3) {
    summary =
      profile.funShare >= 0.5
        ? pickVariant(t, "aiSummary.quick.analysis.month.summary.lightFun", 2, seed)
        : pickVariant(t, "aiSummary.quick.analysis.month.summary.light", 2, seed);
  } else {
    summary = t("aiSummary.quick.analysis.month.summary.minimal");
  }

  const timePattern = monthTimePattern(data, t, seed);
  const categoryBreakdown = monthCategoryBreakdown(data, t, catName, seed);
  const highlights = monthHighlights(data, t, seed);
  const focus = monthFocus(data, t, seed);

  return {
    workOverview,
    timePattern,
    categoryBreakdown,
    highlights,
    focus,
    efficiency,
    suggestion,
    summary,
  };
}

// ───────────────────────── 辅助函数 ─────────────────────────

function formatAppList(apps: QuickUsageEntry[], t: T): string {
  return apps.map((a) => formatAppEntry(a, t)).join(t("aiSummary.quick.analysis.appsJoin"));
}

function formatAppEntry(app: QuickUsageEntry, t: T): string {
  return t("aiSummary.quick.analysis.appEntry", {
    name: app.key,
    duration: formatMinutes(t, app.minutes),
  });
}

function formatMinutes(t: T, m: number): string {
  if (m < 60) return t("aiSummary.quick.duration.minutes", { count: m });
  const h = Math.floor(m / 60);
  const mm = m % 60;
  if (mm === 0) return t("aiSummary.quick.duration.hours", { count: h });
  return t("aiSummary.quick.duration.hoursMinutes", { hours: h, minutes: mm });
}

function formatPercent(p: number): string {
  return `${(p * 100).toFixed(p < 0.1 ? 1 : 0)}%`;
}

function computeVolatility(series: { minutes: number }[]): number {
  const active = series.filter((p) => p.minutes > 0);
  if (active.length < 2) return 0;
  const max = Math.max(...active.map((p) => p.minutes));
  const avg = active.reduce((s, p) => s + p.minutes, 0) / active.length;
  return avg === 0 ? 0 : max / avg;
}
