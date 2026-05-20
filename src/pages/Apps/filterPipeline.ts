import type { AppGroup } from "../../api/hindsight";
import type { AppsFilter } from "./useAppsFilter";

/** 求一个 group 的总秒数 = 所有 member.recentSecs 求和。 */
export function totalRecentSecs(group: AppGroup): number {
  return group.members.reduce((s, m) => s + (m.recentSecs ?? 0), 0);
}

/**
 * 应用筛选 + 排序到 AppGroup 列表，返回**新数组**（不 mutate 入参，配合 React memo）。
 *
 * 应用顺序：
 *   1. `unassignedOnly` 短路（排他）：仅 `categoryId === null` 的 group 通过
 *   2. 否则按 `selectedCategoryIds` 过滤（空数组 = pass-through）
 *   3. `search` 文本匹配 group.displayName 或任一 member.processName（不区分大小写）
 *   4. 按 `sortBy` 排序；`default` 保持入参顺序（依赖 Array.sort 稳定性）
 */
export function applyFilter(groups: AppGroup[], f: AppsFilter): AppGroup[] {
  let out: AppGroup[];

  if (f.unassignedOnly) {
    out = groups.filter((g) => g.categoryId === null);
  } else if (f.selectedCategoryIds.length > 0) {
    const set = new Set(f.selectedCategoryIds);
    out = groups.filter((g) => g.categoryId !== null && set.has(g.categoryId));
  } else {
    out = [...groups];
  }

  // search：空字符串短路，不做无谓 lower-case + 遍历
  const needle = f.search.trim().toLowerCase();
  if (needle.length > 0) {
    out = out.filter((g) => {
      if (g.displayName.toLowerCase().includes(needle)) return true;
      return g.members.some((m) =>
        m.processName.toLowerCase().includes(needle),
      );
    });
  }

  if (f.sortBy !== "default") {
    // 复制后排序——上一步 .filter 已生成新数组，可以原地排
    switch (f.sortBy) {
      case "duration_desc":
        out.sort((a, b) => totalRecentSecs(b) - totalRecentSecs(a));
        break;
      case "duration_asc":
        out.sort((a, b) => totalRecentSecs(a) - totalRecentSecs(b));
        break;
      case "name_asc":
        out.sort((a, b) => a.displayName.localeCompare(b.displayName));
        break;
      case "name_desc":
        out.sort((a, b) => b.displayName.localeCompare(a.displayName));
        break;
    }
  }

  return out;
}
