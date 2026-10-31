import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import type { CategoryTime } from "../api/hindsight";
import { UNGROUPED_COLOR, segmentsBySuper } from "../lib/segments";
import { useCategories } from "../state/categories";
import type { BarLevel } from "../state/statsView";
import { useSuperCategories } from "../state/superCategories";
import { adjustCategoryColor } from "../utils/categoryColor";
import { useIsDark } from "./useTheme";

/** One colored part of a bar, ready to draw. */
export interface BarPart {
  id: string;
  /** Empty when the category cannot be found. */
  name: string;
  color: string;
  secs: number;
}

/** Color of a part whose category cannot be found, for example right after it was deleted. */
const MISSING_COLOR = "var(--cat-fallback, #9ca3af)";

/** Returns a function that turns the segments of one bar into the parts to draw.
 *
 * By category, the parts are the segments in the order given. By super category, they are
 * merged as {@link segmentsBySuper} describes. Either way the parts keep all the seconds, so the
 * bar has the same height at both levels. */
export function useBarParts(level: BarLevel): (segments: CategoryTime[]) => BarPart[] {
  const { t } = useTranslation();
  const { categories, getCategory } = useCategories();
  const { supers } = useSuperCategories();
  const isDark = useIsDark();

  return useCallback(
    (segments) => {
      if (level === "cats") {
        return segments.map((seg) => {
          const cat = getCategory(seg.categoryId);
          return {
            id: seg.categoryId,
            name: cat?.name ?? "",
            color: cat ? adjustCategoryColor(cat.color, isDark) : MISSING_COLOR,
            secs: seg.secs,
          };
        });
      }
      const superById = new Map(supers.map((s) => [s.id, s]));
      return segmentsBySuper(segments, categories, supers).map((part) => {
        const sup = superById.get(part.id);
        return {
          id: part.id,
          name: sup?.name ?? t("categories.super.orphanLabel"),
          color: adjustCategoryColor(sup?.color ?? UNGROUPED_COLOR, isDark),
          secs: part.secs,
        };
      });
    },
    [level, categories, getCategory, supers, isDark, t],
  );
}
