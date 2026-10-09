import { useMemo } from "react";
import type { BreakdownCat, BreakdownSlice } from "./useSuperCategoryBreakdown";

/** 每条 tile 的数据形状；UI 由 InsightTiles 渲染 */
export interface PeriodInsights {
  /** 当期 vs 上期 差（curr - prev）。null = 上期无数据 → A tile 显「—」 */
  diff: { signMinutes: number } | null;
  /** 当期峰值 slot 的展示文本 + 该 slot 分钟数。null = 当期空 */
  peak: { label: string; minutes: number } | null;
  /**
   * 第三 tile：默认 = 主力大类，drill 时 = 当前大类下 top 小分类，
   * 鼠标停在某个应用上时 = 这个应用占所属小类的比例（见 useAppFocus）。
   * kind 区分 label 走哪个 i18n key。
   * undefined = no such tile: a picked category has nothing below it to break down.
   */
  third?:
    | {
        kind: "dominant" | "composition" | "appShare";
        name: string;
        color: string;
        pct: number;
      }
    | null;
}

/**
 * The part of the period the cards cover when the share view narrows down: a drilled
 * super-category, or one category picked in it.
 */
export interface InsightDrill {
  /** The categories whose time counts */
  catIds: ReadonlySet<string>;
  minutes: number;
  /** The same part's minutes in the previous period */
  prevMinutes: number;
  /** The largest category inside, for the breakdown tile; null for a picked category */
  top: BreakdownCat | null;
}

/** Minutes of the picked category, else of the drilled super-category, in one period. */
export function drillMinutes(
  slices: BreakdownSlice[],
  superId: string | null,
  catId: string | null,
): number {
  if (catId !== null) {
    return slices.flatMap((s) => s.cats).find((c) => c.id === catId)?.minutes ?? 0;
  }
  return slices.find((s) => s.id === superId)?.minutes ?? 0;
}

/**
 * What the cards cover: the picked category if there is one, else the drilled super-category,
 * else undefined for the whole period. Neither is found when it has no time this period.
 */
export function insightDrill(
  curr: BreakdownSlice[],
  prev: BreakdownSlice[],
  superId: string | null,
  catId: string | null,
): InsightDrill | undefined {
  if (catId !== null) {
    const cat = curr.flatMap((s) => s.cats).find((c) => c.id === catId);
    if (cat) {
      return {
        catIds: new Set([cat.id]),
        minutes: cat.minutes,
        prevMinutes: drillMinutes(prev, null, cat.id),
        top: null,
      };
    }
  }
  const slice = superId !== null ? curr.find((s) => s.id === superId) : undefined;
  if (!slice) return undefined;
  return {
    catIds: new Set(slice.cats.map((c) => c.id)),
    minutes: slice.minutes,
    prevMinutes: drillMinutes(prev, slice.id, null),
    top: slice.cats[0] ?? null,
  };
}

interface SegmentSource {
  segments: { categoryId: string; secs: number }[];
}

interface UseInsightsArgs<T extends SegmentSource> {
  curr: T[];
  prev: T[];
  buildPeakLabel: (winner: T, idx: number) => string;
  /** 默认模式：currBreakdown.slices[0]。drill 模式下此项被 drill 覆盖 */
  topSlice: BreakdownSlice | null;
  currTotal: number;
  /** drill 模式输入（见 {@link insightDrill}）；undefined = 默认模式 */
  drill?: InsightDrill;
}

function sumMinutes(sources: SegmentSource[]): number {
  // 累秒后取整——与 top-apps / 页面头部总时长同口径（见 CategoryTime.secs 注释）
  let secs = 0;
  for (const src of sources) {
    for (const seg of src.segments) secs += seg.secs;
  }
  return Math.round(secs / 60);
}

/**
 * 三页通用的洞察 hook（v2：支持 drill）。
 *
 * 默认模式：vs 全期上期 / 全期峰值 slot / 全期主力大类
 * drill 模式：vs 该大类上期 / 该大类峰值 slot / 该大类下 top 小分类
 * A category picked in the super-category works the same, without the breakdown tile.
 *
 * 算法都是纯 reduce，N 很小（24 hour / 7 day / 30 day），可在 useMemo 里跑。
 */
export function usePeriodInsights<T extends SegmentSource>(
  args: UseInsightsArgs<T>,
): PeriodInsights {
  const { curr, prev, buildPeakLabel, topSlice, currTotal, drill } = args;

  return useMemo(() => {
    // —— drill 模式 ——
    if (drill) {
      const sliceTotal = drill.minutes;
      if (sliceTotal <= 0) {
        return { diff: null, peak: null, third: null };
      }

      const { catIds } = drill;

      // diff：上期同一范围的 minutes
      const prevSliceMinutes = drill.prevMinutes;
      const diff =
        prevSliceMinutes > 0
          ? { signMinutes: sliceTotal - prevSliceMinutes }
          : null;

      // peak：每个 slot 只 sum 属于该大类的 segments
      let bestIdx = -1;
      let bestSecs = 0;
      for (let i = 0; i < curr.length; i++) {
        let sec = 0;
        for (const seg of curr[i].segments) {
          if (catIds.has(seg.categoryId)) sec += seg.secs;
        }
        if (sec > bestSecs) {
          bestSecs = sec;
          bestIdx = i;
        }
      }
      const peak =
        bestIdx >= 0
          ? {
              label: buildPeakLabel(curr[bestIdx], bestIdx),
              minutes: Math.round(bestSecs / 60),
            }
          : null;

      // third：该大类下 top 小分类（cats 已按 minutes 降序）；选中的小类没有这张卡片
      const topCat = drill.top;
      const third = topCat
        ? {
            kind: "composition" as const,
            name: topCat.name,
            color: topCat.color,
            pct: Math.round((topCat.minutes / sliceTotal) * 100),
          }
        : undefined;

      return { diff, peak, third };
    }

    // —— 默认模式 ——
    if (currTotal <= 0) {
      return { diff: null, peak: null, third: null };
    }

    const prevTotal = sumMinutes(prev);
    const diff =
      prevTotal > 0 ? { signMinutes: currTotal - prevTotal } : null;

    let bestIdx = -1;
    let bestSecs = 0;
    for (let i = 0; i < curr.length; i++) {
      let sec = 0;
      for (const seg of curr[i].segments) sec += seg.secs;
      if (sec > bestSecs) {
        bestSecs = sec;
        bestIdx = i;
      }
    }
    const peak =
      bestIdx >= 0
        ? {
            label: buildPeakLabel(curr[bestIdx], bestIdx),
            minutes: Math.round(bestSecs / 60),
          }
        : null;

    const third = topSlice
      ? {
          kind: "dominant" as const,
          name: topSlice.name,
          color: topSlice.color,
          pct: Math.round((topSlice.minutes / currTotal) * 100),
        }
      : null;

    return { diff, peak, third };
  }, [curr, prev, buildPeakLabel, topSlice, currTotal, drill]);
}
