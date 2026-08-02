/**
 * 「输入应用名」的自动补全候选:候选池构建 + 输入过滤。
 *
 * 两个调用方共用同一套规则,避免两处各写一份、行为悄悄分叉:
 * - 分类页给分类添加应用(选中即把该进程归入本分类)
 * - 设置 → 隐私 的应用关键词(选中即把该进程名填成跳过截图的关键词)
 *
 * 候选来源都是 `list_app_groups` —— Hindsight 真实采集过的进程,
 * 所以补全出来的名字一定能匹配上后续的记录。
 */

import type { AppGroup } from "../api/hindsight";
import { displayAppName } from "../utils/displayName";

/** 自动补全候选:Hindsight 记录过的一个进程 + 它的归属信息。 */
export interface AppSuggestion {
  process: string;
  display: string;
  /** 所在应用组的显示名(与 display 不同时作为附加匹配字段) */
  groupName: string;
  /** 现属分类 id;null = 未分类 */
  categoryId: string | null;
  /** 近 7 天用时,用作默认排序(常用的排前面) */
  recentSecs: number;
}

/** 面板最多渲染的候选数(超出滚动)。 */
export const SUGGEST_MAX = 8;

/**
 * 摊平应用组 → 候选池。
 *
 * 排序:**未分类优先**,组内再按近 7 天用时降序。未分类优先是因为
 * "添加应用"的最高频场景就是收编刚冒出来的新应用;已分类的仍在池里,
 * 靠输入过滤能选到(分类页选中即挪移)。
 *
 * `exclude` 里的进程名不进池(忽略大小写):分类页排除已在本分类的,
 * 隐私页排除已经加过的关键词。
 */
export function buildAppSuggestions(
  groups: AppGroup[],
  exclude?: Iterable<string>,
): AppSuggestion[] {
  const skip = new Set<string>();
  for (const e of exclude ?? []) skip.add(e.trim().toLowerCase());
  return groups
    .flatMap((g) =>
      g.members.map((m) => ({
        process: m.processName,
        display: displayAppName(m.processName),
        groupName: g.displayName,
        categoryId: g.categoryId,
        recentSecs: m.recentSecs,
      })),
    )
    .filter((s) => !skip.has(s.process.trim().toLowerCase()))
    .sort(
      (a, b) =>
        Number(a.categoryId !== null) - Number(b.categoryId !== null) ||
        b.recentSecs - a.recentSecs,
    );
}

/**
 * 输入过滤:进程名 / 显示名 / 组名任一子串命中即可,忽略大小写。
 * 空输入 = 不过滤(直接给排序后的前 `max` 个,让用户不打字也能挑)。
 */
export function filterAppSuggestions(
  pool: AppSuggestion[],
  query: string,
  max: number = SUGGEST_MAX,
): AppSuggestion[] {
  const q = query.trim().toLowerCase();
  const hit = q
    ? pool.filter(
        (s) =>
          s.process.toLowerCase().includes(q) ||
          s.display.toLowerCase().includes(q) ||
          s.groupName.toLowerCase().includes(q),
      )
    : pool;
  return hit.slice(0, max);
}
