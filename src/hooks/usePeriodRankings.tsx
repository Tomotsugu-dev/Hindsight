import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { AppIcon } from "../components/AppIcon/AppIcon";
import { AppStack } from "../components/AppStack/AppStack";
import { useCategories } from "../state/categories";
import type { AppUsage } from "../api/hindsight";
import { displayAppName } from "../utils/displayName";
import { displayCategoryName } from "../utils/categoryName";
import type { RankedItem } from "../components/RankedList/RankedList";

/** 排行所需的每个数据源（每小时槽位 / 每日汇总）通用形状：
 *  只关心其 segments 数组的 categoryId + minutes。 */
interface SegmentSource {
  segments: { categoryId: string; minutes: number }[];
}

/**
 * 把 hours / days 数据 + apps 数据 → categoryRanks + appRanks 的统一计算。
 *
 * 抽自 Today/Week/Month 三页的 `useMemo<RankedItem[]>` 块（三处一字不差）。
 */
export function usePeriodRankings(
  segmentSources: SegmentSource[],
  apps: AppUsage[],
): { categoryRanks: RankedItem[]; appRanks: RankedItem[] } {
  const { t } = useTranslation();
  const { categories, getCategory } = useCategories();

  const categoryRanks = useMemo<RankedItem[]>(() => {
    const totals = new Map<string, number>();
    for (const src of segmentSources) {
      for (const seg of src.segments) {
        totals.set(
          seg.categoryId,
          (totals.get(seg.categoryId) ?? 0) + seg.minutes,
        );
      }
    }
    const topAppsByCat = new Map<string, string[]>();
    for (const a of apps) {
      if (!a.categoryId) continue;
      const list = topAppsByCat.get(a.categoryId) ?? [];
      // AppStack 拿这串去查图标，必须用 iconProcess（合并组里的稳定代表名）；
      // a.process 是组的 display_name，可能跟 app_icons 表里的 key 不一致。
      list.push(a.iconProcess);
      topAppsByCat.set(a.categoryId, list);
    }
    return categories
      .map((c) => ({
        id: c.id,
        name: displayCategoryName(c, t),
        color: c.color,
        minutes: totals.get(c.id) ?? 0,
        extras: (
          <AppStack
            apps={topAppsByCat.get(c.id) ?? []}
            fallbackColor={c.color}
          />
        ),
      }))
      .filter((c) => c.minutes > 0)
      .sort((a, b) => b.minutes - a.minutes);
  }, [segmentSources, apps, categories, t]);

  const appRanks = useMemo<RankedItem[]>(() => {
    return apps.map((a) => {
      const cat = getCategory(a.categoryId);
      const color = cat?.color ?? "#94a3b8";
      return {
        id: a.process,
        name: displayAppName(a.process),
        subtitle: cat ? displayCategoryName(cat, t) : undefined,
        color,
        minutes: a.minutes,
        leading: <AppIcon processName={a.iconProcess} fallbackColor={color} size={26} />,
        categoryId: a.categoryId,
      };
    });
  }, [apps, getCategory, t]);

  return { categoryRanks, appRanks };
}
