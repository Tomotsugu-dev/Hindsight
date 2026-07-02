import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useCategories } from "../state/categories";
import { useSuperCategories } from "../state/superCategories";
import {
  displayCategoryName,
  displaySuperCategoryName,
} from "../utils/categoryName";

/** 用于 donut 展开层的小分类条目（同大类内按 minutes 降序） */
export interface BreakdownCat {
  id: string;
  name: string;
  /** 子分类本身的颜色（用户在 AppearancePicker 设的） */
  color: string;
  minutes: number;
}

/** donut 顶层切片：一个大类（或 orphan 兜底"未归入"） */
export interface BreakdownSlice {
  /** super_category id；orphan 用 sentinel "__orphan__" */
  id: string;
  /** 展示名（builtin 大类走 i18n） */
  name: string;
  /** 大类色 —— donut 切片填这个；orphan 走中性暖灰 */
  color: string;
  /** lucide icon 名（PieView 行首图标用）；orphan 用 Folder 兜底 */
  icon: string;
  minutes: number;
  /** 该大类下的小分类（按 minutes 降序）；空数组表示该大类下没活动 */
  cats: BreakdownCat[];
}

const ORPHAN_KEY = "__orphan__";
/** stone-400 暖中性灰 —— 比原来的 slate-400 (#94a3b8) 暖一点，
 *  避免"未归入"行在 donut/legend 里太死板冷淡 */
const ORPHAN_COLOR = "#a8a29e";

/** 输入形状：只关心 id + minutes，跟 RankedItem 兼容（cast 即可传入） */
interface CategoryMinutesInput {
  id: string;
  minutes: number;
}

/** 把 HourSlot[] / DaySummary[] 等带 `segments` 的源汇总成 [{id, minutes}, ...]。
 *  给 useSuperCategoryBreakdown 当 input；Today 用 hours, Week/Month 用 days。 */
export function catMinutesFromSegments(
  sources: { segments: { categoryId: string; secs: number }[] }[],
): CategoryMinutesInput[] {
  // 累秒后统一取整——逐桶取整再相加会系统性偏差（碎片使用越多偏得越多），
  // 与 top-apps 的"先加总后取整"口径保持一致。
  const totalSecs = new Map<string, number>();
  for (const src of sources) {
    for (const seg of src.segments) {
      totalSecs.set(seg.categoryId, (totalSecs.get(seg.categoryId) ?? 0) + seg.secs);
    }
  }
  return Array.from(totalSecs, ([id, secs]) => ({ id, minutes: Math.round(secs / 60) }));
}

/**
 * 把 categoryRanks（小分类 → 分钟数）按 super_category_id 聚合成 donut 切片。
 *
 * - builtin 分类（如 hidden）**不计入** —— 它们不参与时长统计语义
 * - 没归到任何 super 的小分类聚合到 orphan slice
 * - 各级（slice / cat）都按 minutes 降序，便于 chip 取 top-1 + popover legend 直接渲染
 *
 * 性能：纯前端 reduce，N=分类数（通常 <20），跟随父组件 useMemo 依赖触发。
 */
export function useSuperCategoryBreakdown(catMinutes: CategoryMinutesInput[]): {
  slices: BreakdownSlice[];
  total: number;
} {
  const { t } = useTranslation();
  const { categories } = useCategories();
  const { supers } = useSuperCategories();

  return useMemo(() => {
    const catById = new Map(categories.map((c) => [c.id, c]));
    const supById = new Map(supers.map((s) => [s.id, s]));

    const sliceMap = new Map<string, BreakdownSlice>();
    // 预放真大类的空 slice，保证 supers 列表里没活动的也至少能在内部状态里 lookup
    // （最后会被 filter minutes > 0 清掉）
    for (const sup of supers) {
      sliceMap.set(sup.id, {
        id: sup.id,
        name: displaySuperCategoryName(sup, t),
        color: sup.color,
        icon: sup.icon,
        minutes: 0,
        cats: [],
      });
    }

    for (const cm of catMinutes) {
      if (cm.minutes <= 0) continue;
      const cat = catById.get(cm.id);
      if (!cat) continue;
      // builtin（hidden 等）不进 donut —— 它们的"占比"没语义
      if (cat.builtin) continue;

      const superId =
        cat.superCategoryId && supById.has(cat.superCategoryId)
          ? cat.superCategoryId
          : ORPHAN_KEY;

      let slice = sliceMap.get(superId);
      if (!slice) {
        slice = {
          id: ORPHAN_KEY,
          name: t("categories.super.orphanLabel"),
          color: ORPHAN_COLOR,
          icon: "Folder",
          minutes: 0,
          cats: [],
        };
        sliceMap.set(superId, slice);
      }
      slice.minutes += cm.minutes;
      slice.cats.push({
        id: cat.id,
        name: displayCategoryName(cat, t),
        color: cat.color,
        minutes: cm.minutes,
      });
    }

    for (const slice of sliceMap.values()) {
      slice.cats.sort((a, b) => b.minutes - a.minutes);
    }

    const slices = Array.from(sliceMap.values())
      .filter((s) => s.minutes > 0)
      .sort((a, b) => b.minutes - a.minutes);
    const total = slices.reduce((sum, s) => sum + s.minutes, 0);
    return { slices, total };
  }, [catMinutes, categories, supers, t]);
}
