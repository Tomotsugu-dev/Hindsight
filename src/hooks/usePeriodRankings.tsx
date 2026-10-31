import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { AppIcon } from "../components/AppIcon/AppIcon";
import { AppStack } from "../components/AppStack/AppStack";
import { useCategories } from "../state/categories";
import { useSuperCategories } from "../state/superCategories";
import type { BarLevel } from "../state/statsView";
import type { AppUsage } from "../api/hindsight";
import { UNGROUPED_COLOR, UNGROUPED_ID, rankByLevel, superIdOf } from "../lib/segments";
import { displayAppName } from "../utils/displayName";
import type { RankedItem } from "../components/RankedList/RankedList";

/** Shape shared by every ranking source (hour slots / day summaries): only the categoryId and
 *  secs of each segment are used. */
interface SegmentSource {
  segments: { categoryId: string; secs: number }[];
}

/**
 * 把 hours / days 数据 + apps 数据 → categoryRanks + appRanks 的统一计算。
 *
 * 抽自 Today/Week/Month 三页的 `useMemo<RankedItem[]>` 块（三处一字不差）。
 *
 * At the super category level, the category ranking lists super categories, and each app is
 * labeled and colored by its super category, to match bars colored the same way.
 */
export function usePeriodRankings(
  segmentSources: SegmentSource[],
  apps: AppUsage[],
  level: BarLevel = "cats",
): { categoryRanks: RankedItem[]; appRanks: RankedItem[] } {
  const { t } = useTranslation();
  const { categories, getCategory } = useCategories();
  const { supers } = useSuperCategories();
  const ungroupedName = t("categories.super.orphanLabel");

  const categoryRanks = useMemo<RankedItem[]>(() => {
    // An app is listed under every category it has time in, ranked by that time:
    // Chrome shows under Video when a website rule moves Bilibili there.
    const ranks = rankByLevel(level, segmentSources, apps, categories, supers);
    const rows =
      level === "supers"
        ? [...supers, { id: UNGROUPED_ID, name: ungroupedName, color: UNGROUPED_COLOR }]
        : categories;
    return rows
      .map((row) => ({
        id: row.id,
        name: row.name,
        color: row.color,
        minutes: Math.round((ranks.get(row.id)?.secs ?? 0) / 60),
        extras: (
          <AppStack
            apps={ranks.get(row.id)?.iconProcesses ?? []}
            fallbackColor={row.color}
          />
        ),
      }))
      .filter((c) => c.minutes > 0)
      .sort((a, b) => b.minutes - a.minutes);
  }, [segmentSources, apps, categories, supers, level, ungroupedName]);

  const appRanks = useMemo<RankedItem[]>(() => {
    const superById = new Map(supers.map((s) => [s.id, s]));
    const superIds = new Set(superById.keys());
    return apps.map((a) => {
      const cat = getCategory(a.categoryId);
      let label = cat?.name;
      let color = cat?.color ?? "#94a3b8";
      if (level === "supers") {
        const sup = superById.get(superIdOf(cat, superIds));
        label = sup?.name ?? ungroupedName;
        color = sup?.color ?? UNGROUPED_COLOR;
      }
      return {
        id: a.groupId,
        name: displayAppName(a.displayName),
        subtitle: label,
        color,
        minutes: a.minutes,
        leading: <AppIcon processName={a.iconProcess} fallbackColor={color} size={26} />,
        categoryId: a.categoryId,
        iconProcess: a.iconProcess,
        byCategory: a.byCategory,
      };
    });
  }, [apps, getCategory, supers, level, ungroupedName]);

  return { categoryRanks, appRanks };
}
