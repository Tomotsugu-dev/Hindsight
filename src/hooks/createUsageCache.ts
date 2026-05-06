import { useCallback, useEffect, useRef, useState } from "react";
import { useCategories } from "../state/categories";

/**
 * useDayCache / useWeekCache / useMonthCache 的工厂：抽 95% 同构的滑窗缓存
 * + 预取相邻 offset + 当前期轮询 + 切设备/分类清缓存。
 *
 * 各 hook 仅在三处不同：fetch 函数、空值常量、轮询间隔。统一为同一份语义后：
 * - 缓存按 offset 存（offset > 0 不允许）
 * - 切 deviceId / categories 引用变化时清缓存重新拉（让分类页改动立刻反映）
 * - currentOffset 变 → 预取 [-1, 0, +1]
 * - currentOffset === 0 时按 pollInterval 轮询当前期
 */
export interface UsageCacheConfig<TData> {
  /** 拉一期数据，offset 是相对当前期的整数（0=当前期，-1=上一期） */
  fetch: (offset: number, deviceId?: string) => Promise<TData>;
  /** 缓存未命中或 offset > 0 时的占位值 */
  emptyValue: TData;
  /** 当前期数据轮询间隔（ms）；offset !== 0 不轮询 */
  pollInterval: number;
}

export function createUsageCache<TData>(config: UsageCacheConfig<TData>) {
  const { fetch, emptyValue, pollInterval } = config;

  return function useUsageCache(currentOffset: number, deviceId?: string) {
    const { categories } = useCategories();
    const [cache, setCache] = useState<Map<number, TData>>(new Map());
    const inFlightRef = useRef<Set<number>>(new Set());

    const fetchOne = useCallback(
      async (offset: number) => {
        if (offset > 0) return;
        if (inFlightRef.current.has(offset)) return;
        inFlightRef.current.add(offset);
        try {
          const data = await fetch(offset, deviceId);
          setCache((prev) => {
            const next = new Map(prev);
            next.set(offset, data);
            return next;
          });
        } catch {
          // 查询失败静默——UI 由 emptyValue 兜底，错误细节后端日志已记
        } finally {
          inFlightRef.current.delete(offset);
        }
      },
      [deviceId],
    );

    // 切设备 / categories 引用变化（CategoriesProvider 每次 refresh 后都换新数组）→
    // 清空缓存重新拉。这样分类页指派 / 配对操作完，三页立刻反映。
    useEffect(() => {
      setCache(new Map());
      inFlightRef.current.clear();
    }, [deviceId, categories]);

    // categories 进 deps：CategoriesProvider 初次 mount 是空数组，
    // 数据回来后会换新引用 → 上面的清缓存 effect 已触发把刚到的数据清掉，
    // 不带 categories 的话就再也不会补一发，UI 卡空白。
    useEffect(() => {
      fetchOne(currentOffset - 1);
      fetchOne(currentOffset);
      fetchOne(currentOffset + 1);
    }, [currentOffset, fetchOne, categories]);

    useEffect(() => {
      if (currentOffset !== 0) return;
      const t = setInterval(() => {
        inFlightRef.current.delete(0);
        fetchOne(0);
      }, pollInterval);
      return () => clearInterval(t);
    }, [currentOffset, fetchOne]);

    const get = useCallback(
      (offset: number): TData => cache.get(offset) ?? emptyValue,
      [cache],
    );

    return { get };
  };
}
