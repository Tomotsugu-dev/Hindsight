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
    // cache 的同步镜像 —— fetchOne 里用 ref 比 state 闭包稳定（避免拿到过时 cache）。
    const cacheRef = useRef<Map<number, TData>>(cache);

    const fetchOne = useCallback(
      // `force` = true 时无视已 cached，强制重发（给 polling + 当前期用）；
      //     默认 false：邻居预取看到 cache 已有就 skip，避免重 fetch 把
      //     UI 已经渲染好的柱子又"漂"一下（cross-device sync / 延迟 seal_session
      //     等会让同一个 SQL 在不同时刻返回略不同结果）。
      async (offset: number, force = false) => {
        if (offset > 0) return;
        if (inFlightRef.current.has(offset)) return;
        if (!force && cacheRef.current.has(offset)) return;
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
      cacheRef.current = new Map();
      inFlightRef.current.clear();
    }, [deviceId, categories]);

    // cache state 变化时同步到 ref，让 fetchOne 里的 has() 判断永远拿到最新值
    useEffect(() => {
      cacheRef.current = cache;
    }, [cache]);

    // categories 进 deps：CategoriesProvider 初次 mount 是空数组，
    // 数据回来后会换新引用 → 上面的清缓存 effect 已触发把刚到的数据清掉，
    // 不带 categories 的话就再也不会补一发，UI 卡空白。
    //
    // force 策略（关键：决定切日时柱子稳不稳）：
    //   - currentOffset === 0 (今天)：force=true。今天数据时时刻刻在变（capture 一直
    //     在写），切回来时强制拿最新，避免看到几分钟前的状态。
    //   - currentOffset < 0 (历史)：force=false。历史天理论上不该变（除非 cross-device
    //     sync 拉到新行 / 延迟 seal_session）；cache 有就直接用，**不重发** ——
    //     之前误把"切到昨天也 force=true"导致 cached 显示后 50ms 又被新 fetch 覆盖，
    //     就是用户看到的"16h 柱子突然变长 / 变短"。
    //   - 邻居（currentOffset ± 1）：永远 force=false，cache 有就跳过预取。
    useEffect(() => {
      fetchOne(currentOffset - 1);
      fetchOne(currentOffset, currentOffset === 0);
      fetchOne(currentOffset + 1);
    }, [currentOffset, fetchOne, categories]);

    useEffect(() => {
      if (currentOffset !== 0) return;
      const t = setInterval(() => {
        inFlightRef.current.delete(0);
        fetchOne(0, true);
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
