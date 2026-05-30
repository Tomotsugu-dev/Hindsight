import { useCallback, useEffect, useState } from "react";

/** 排序方式枚举：default = 保持入参顺序（PairingSection 现有的 device-sort + 未指派靠前）。 */
export type AppsSortBy =
  | "default"
  | "duration_desc"
  | "duration_asc"
  | "name_asc"
  | "name_desc";

const VALID_SORT_BYS: AppsSortBy[] = [
  "default",
  "duration_desc",
  "duration_asc",
  "name_asc",
  "name_desc",
];

export interface AppsFilter {
  search: string;
  /** 选中的分类 id 数组；空数组 = 不限分类（pass-through）。 */
  selectedCategoryIds: string[];
  /** 排他模式：true 时只显示 categoryId === null 的行，其他过滤条件失效。 */
  unassignedOnly: boolean;
  sortBy: AppsSortBy;
}

export const DEFAULT_APPS_FILTER: AppsFilter = {
  search: "",
  selectedCategoryIds: [],
  unassignedOnly: false,
  sortBy: "default",
};

const STORAGE_KEY = "hindsight.apps.filter";

/**
 * Type-safe revival from localStorage：未知字段 / 坏值都回默认。
 * 故意不抛错——损坏的 JSON 不应让用户的整个 /apps 页崩溃，silently reset 更友好。
 */
function loadFromStorage(): AppsFilter {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_APPS_FILTER;
    const parsed = JSON.parse(raw) as Partial<AppsFilter>;
    return {
      search: typeof parsed.search === "string" ? parsed.search : "",
      selectedCategoryIds: Array.isArray(parsed.selectedCategoryIds)
        ? parsed.selectedCategoryIds.filter((x): x is string => typeof x === "string")
        : [],
      unassignedOnly: parsed.unassignedOnly === true,
      sortBy:
        typeof parsed.sortBy === "string" && VALID_SORT_BYS.includes(parsed.sortBy)
          ? (parsed.sortBy)
          : "default",
    };
  } catch {
    return DEFAULT_APPS_FILTER;
  }
}

function saveToStorage(filter: AppsFilter): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(filter));
  } catch {
    // localStorage 满 / 隐私模式 / 等：忽略，session 内仍然有效
  }
}

/**
 * 管理 `/apps` 页的筛选 + 排序状态，自动持久化到 localStorage。
 * 沿用 [`deviceFilter.tsx`] 同款"读取在 lazy initializer / 写入在 setter"模式。
 */
export function useAppsFilter() {
  const [filter, setFilter] = useState<AppsFilter>(() => loadFromStorage());

  // 任何子字段变更都触发整体保存——简化心智模型，多写一次 localStorage 几乎零成本
  useEffect(() => {
    saveToStorage(filter);
  }, [filter]);

  const setSearch = useCallback((search: string) => {
    setFilter((f) => ({ ...f, search }));
  }, []);

  /** 切换某个真分类的选中状态。会自动取消 unassignedOnly（互斥模式）。 */
  const toggleCategory = useCallback((id: string) => {
    setFilter((f) => {
      const has = f.selectedCategoryIds.includes(id);
      return {
        ...f,
        selectedCategoryIds: has
          ? f.selectedCategoryIds.filter((x) => x !== id)
          : [...f.selectedCategoryIds, id],
        unassignedOnly: false,
      };
    });
  }, []);

  /** 切换「未分类」排他模式。开启时清空 selectedCategoryIds，关闭时不动其他。 */
  const toggleUnassignedOnly = useCallback(() => {
    setFilter((f) =>
      f.unassignedOnly
        ? { ...f, unassignedOnly: false }
        : { ...f, unassignedOnly: true, selectedCategoryIds: [] },
    );
  }, []);

  /** 「全部」按钮：清空所有分类筛选条件，回到默认 pass-through。 */
  const resetCategories = useCallback(() => {
    setFilter((f) => ({ ...f, selectedCategoryIds: [], unassignedOnly: false }));
  }, []);

  const setSortBy = useCallback((sortBy: AppsSortBy) => {
    setFilter((f) => ({ ...f, sortBy }));
  }, []);

  const clearAll = useCallback(() => {
    setFilter(DEFAULT_APPS_FILTER);
  }, []);

  /** 是否当前有任何"激活"的筛选条件（用来决定是否显示 noResults 的"清除筛选"按钮）。 */
  const isFiltering =
    filter.search.length > 0 ||
    filter.selectedCategoryIds.length > 0 ||
    filter.unassignedOnly;

  return {
    filter,
    setSearch,
    toggleCategory,
    toggleUnassignedOnly,
    resetCategories,
    setSortBy,
    clearAll,
    isFiltering,
  };
}
