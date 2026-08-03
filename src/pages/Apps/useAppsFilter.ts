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
  /** 默认只看未指派：这一页本质是"把应用归类"的工作台，该出现在这里的是还
   *  没处理的应用。系统跑过的每个进程都会被记下来，半年前开过一分钟的东西
   *  也永远占着位置——归了类就自动从列表里退场，比手动删干净且不丢历史。
   *  想看全部：清掉筛选即可。 */
  unassignedOnly: true,
  sortBy: "default",
};

const STORAGE_KEY = "hindsight.apps.filter";

/** 存储结构版本。老用户的 localStorage 里存着 `unassignedOnly: false`，
 *  光改默认值对他们不生效——而列表最乱的恰恰是这批人。带版本号做一次性
 *  迁移：读到旧版就把开关拉起来一次，之后完全尊重用户自己的选择。 */
const FILTER_SCHEMA_VERSION = 2;

interface StoredFilter extends Partial<AppsFilter> {
  v?: number;
}

/**
 * Type-safe revival from localStorage：未知字段 / 坏值都回默认。
 * 故意不抛错——损坏的 JSON 不应让用户的整个 /apps 页崩溃，silently reset 更友好。
 */
function loadFromStorage(): AppsFilter {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_APPS_FILTER;
    const parsed = JSON.parse(raw) as StoredFilter;
    // 旧版存档：把"只看未指派"打开一次(见 FILTER_SCHEMA_VERSION)
    const legacy = parsed.v !== FILTER_SCHEMA_VERSION;
    return {
      search: typeof parsed.search === "string" ? parsed.search : "",
      selectedCategoryIds: Array.isArray(parsed.selectedCategoryIds)
        ? parsed.selectedCategoryIds.filter((x): x is string => typeof x === "string")
        : [],
      unassignedOnly: legacy ? true : parsed.unassignedOnly === true,
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
    // 带上版本号,否则每次读都会被当成旧版存档、把用户关掉的开关又打开
    const payload: StoredFilter = { ...filter, v: FILTER_SCHEMA_VERSION };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
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
