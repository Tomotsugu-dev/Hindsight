import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AppUsage } from "../api/hindsight";
import type { RingFocusApp } from "../components/PieView/ringItems";
import { useCategories } from "../state/categories";
import { displayCategoryName } from "../utils/categoryName";
import { displayAppName } from "../utils/displayName";
import { useAppDetail, type DetailScope } from "./useAppDetail";
import type { PeriodInsights } from "./usePeriodInsights";
import type { BreakdownSlice } from "./useSuperCategoryBreakdown";

/** 鼠标在一行上停这么久才算选中它；快速扫过一串应用时圆环不跟着乱跳 */
const SETTLE_MS = 120;

interface Args {
  /** 鼠标停着的应用行的 id（分组 ID）；null = 没停在任何应用上 */
  hoverId: string | null;
  /** 占比视图才突出应用；时段视图下传 false，什么都不算也不拉 */
  enabled: boolean;
  /** 本期每个应用的时长，按时长从多到少 */
  apps: AppUsage[];
  /** 上期每个应用的时长；上期没出现的应用按 0 算 */
  prevApps: AppUsage[];
  /** 上期总分钟数；为 0 时「较上期」显示 —，跟总览一样 */
  prevTotal: number;
  /** 本期圆环的大类切片，用来取应用所属小类的分钟数 */
  slices: BreakdownSlice[];
  scope: DetailScope;
  offset: number;
  deviceId?: string;
  /** 峰值那根柱子的 key（日="0".."23"，周/月="YYYY-MM-DD"）→ 卡片上的文字 */
  peakLabel: (key: string) => string;
  /** 周、月统计：日均只算已经过完的天（早于今天的），上期按 prevDays 天平均 */
  avg?: { prevDays: number };
}

export interface AppFocus {
  /** 交给圆环：展开哪个应用；null = 没突出应用 */
  focusApp: RingFocusApp | null;
  /** 交给统计卡片：这个应用自己的「较上期 / 峰值 / 占所属分类」；null = 没突出应用 */
  insights: PeriodInsights | null;
  /** 周、月统计卡片的日均和上期日均；null = 还在拉这个应用每天的时长 */
  avgMinutes?: number | null;
  prevAvgMinutes?: number | null;
  /** 统计卡片左边色条用的颜色 */
  color?: string;
}

/** 本地日期 → "YYYY-MM-DD"，跟按天的柱子 key 同格式 */
function todayKey(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/**
 * 鼠标停在应用排行某一行时，算出圆环和统计卡片要显示的这个应用的数据。
 * 峰值和日均要这个应用每个小时 / 每天的时长，跟点开应用详情用同一个查询和缓存。
 */
export function useAppFocus({
  hoverId,
  enabled,
  apps,
  prevApps,
  prevTotal,
  slices,
  scope,
  offset,
  deviceId,
  peakLabel,
  avg,
}: Args): AppFocus {
  const { t } = useTranslation();
  const { getCategory } = useCategories();

  // 停住 SETTLE_MS 才换；从一行移到下一行中间那一下 null 也就跳过了
  const [settledId, setSettledId] = useState<string | null>(null);
  useEffect(() => {
    const timer = window.setTimeout(() => setSettledId(hoverId), SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [hoverId]);

  const app = enabled && settledId ? (apps.find((a) => a.groupId === settledId) ?? null) : null;
  const { detail } = useAppDetail(scope, offset, app?.groupId ?? null, deviceId);

  return useMemo<AppFocus>(() => {
    if (!app) return { focusApp: null, insights: null };

    const cat = getCategory(app.categoryId);
    const color = cat?.color ?? "#94a3b8";
    const catMinutes =
      slices.flatMap((s) => s.cats).find((c) => c.id === app.categoryId)?.minutes ??
      apps.filter((a) => a.categoryId === app.categoryId).reduce((sum, a) => sum + a.minutes, 0);

    const focusApp: RingFocusApp = {
      groupId: app.groupId,
      name: displayAppName(app.displayName),
      iconProcess: app.iconProcess,
      categoryId: app.categoryId,
      color,
      minutes: app.minutes,
    };

    const prevMinutes = prevApps.find((a) => a.groupId === app.groupId)?.minutes ?? 0;
    const buckets = detail?.buckets ?? [];
    const best = buckets.reduce<{ key: string; secs: number } | null>(
      (top, b) => (b.secs > 0 && (!top || b.secs > top.secs) ? b : top),
      null,
    );
    const insights: PeriodInsights = {
      diff: prevTotal > 0 ? { signMinutes: app.minutes - prevMinutes } : null,
      peak: best ? { label: peakLabel(best.key), minutes: Math.round(best.secs / 60) } : null,
      third:
        cat && catMinutes > 0
          ? {
              kind: "appShare",
              name: displayCategoryName(cat, t),
              color,
              pct: Math.round((app.minutes / catMinutes) * 100),
            }
          : null,
    };

    if (!avg) return { focusApp, insights, color };

    // 日均只算已经过完的天，跟总览的日均同口径
    let avgMinutes: number | null | undefined = null;
    if (detail) {
      const today = todayKey();
      const done = buckets.filter((b) => b.key < today);
      avgMinutes =
        done.length > 0
          ? Math.round(done.reduce((sum, b) => sum + b.secs, 0) / 60 / done.length)
          : undefined;
    }
    const prevAvgMinutes =
      prevTotal > 0 && avg.prevDays > 0 ? Math.round(prevMinutes / avg.prevDays) : undefined;
    return { focusApp, insights, avgMinutes, prevAvgMinutes, color };
  }, [app, apps, prevApps, prevTotal, slices, detail, peakLabel, avg, getCategory, t]);
}
