import { useCallback, useEffect, useRef, useState } from "react";
import { useCategories } from "../state/categories";
import { lruInsert } from "../utils/lru";

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
  /**
   * cache 上限（按插入序 evict）。默认 8 = 覆盖 currentOffset ±1 预取（3）+
   * 5 个最近翻过的历史位置。DayData ~30–80 KB / WeekData / MonthData 更大；
   * 8 让最坏占用上界从"无限增长"压回固定 MB 量级。
   */
  maxCacheSize?: number;
}

const DEFAULT_MAX_CACHE_SIZE = 8;

export function createUsageCache<TData>(config: UsageCacheConfig<TData>) {
  const { fetch, emptyValue, pollInterval } = config;
  const maxCacheSize = config.maxCacheSize ?? DEFAULT_MAX_CACHE_SIZE;

  return function useUsageCache(currentOffset: number, deviceId?: string) {
    const { categories } = useCategories();
    const [cache, setCache] = useState<Map<number, TData>>(new Map());
    const inFlightRef = useRef<Set<number>>(new Set());
    // cache 的同步镜像 —— fetchOne 里用 ref 比 state 闭包稳定（避免拿到过时 cache）。
    const cacheRef = useRef<Map<number, TData>>(cache);
    // scope 代际：每次切设备/分类 +1。fetchOne 在 await 前快照，回来后比对——
    // 不一致说明这份数据属于已经切走的旧 scope，丢弃，避免写进新 scope 的缓存。
    const scopeRef = useRef(0);

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
        const scope = scopeRef.current; // await 前快照当前 scope 代际
        try {
          const data = await fetch(offset, deviceId);
          // 解析期间切了设备/分类 → scope 已变，这份数据属于旧 scope，丢弃。
          // 历史 offset 用 force=false 不会自纠，不丢就会把旧设备数据算到新设备头上。
          if (scope !== scopeRef.current) return;
          setCache((prev) => {
            const next = new Map(prev);
            lruInsert(next, offset, data, maxCacheSize);
            return next;
          });
        } catch {
          // 查询失败静默——UI 由 emptyValue 兜底，错误细节后端日志已记
        } finally {
          // 仅清理仍属当前 scope 的在途标记；scope 已切换时 inFlight 已被清空 effect 重置，
          // 误删会把新 scope 刚加入的同 offset 标记清掉，放行多余的重复 fetch。
          if (scope === scopeRef.current) inFlightRef.current.delete(offset);
        }
      },
      [deviceId],
    );

    // 切设备 / categories 引用变化（CategoriesProvider 每次 refresh 后都换新数组）→
    // 清空缓存重新拉。这样分类页指派 / 配对操作完，三页立刻反映。
    useEffect(() => {
      scopeRef.current += 1; // 代际 +1：让仍在解析的旧 scope fetch 作废
      setCache(new Map());
      cacheRef.current = new Map();
      inFlightRef.current.clear();
    }, [deviceId, categories]);

    // cache state 变化时同步到 ref，让 fetchOne 里的 has() 判断永远拿到最新值
    useEffect(() => {
      cacheRef.current = cache;
    }, [cache]);

    // 跨午夜失效：缓存按**相对 offset** 键控，而后端按查询时刻的 now 解析 offset。
    // 常驻 app 挂过夜后，昨天缓存的 offset=-1 内容其实是"前天"——不清的话第二天
    // 点「昨天」看到的是前天的数据。记录写入日，发现本地日期变了就整体作废重拉。
    // 返回 true = 刚发生翻转（调用方需要触发重拉）。
    const dayKeyRef = useRef(new Date().toDateString());
    const invalidateOnRollover = useCallback((): boolean => {
      const key = new Date().toDateString();
      if (key === dayKeyRef.current) return false;
      dayKeyRef.current = key;
      scopeRef.current += 1; // 在途旧 fetch 一并作废
      setCache(new Map());
      cacheRef.current = new Map();
      inFlightRef.current.clear();
      return true;
    }, []);

    // 翻转检测不能只挂在 offset=0 的轮询上：用户停在历史 offset 过夜时那个轮询
    // 根本不跑，日期标签（每次渲染重算）已经翻了、数据还是旧 offset 语义。
    // 独立低频定时器常驻检查；翻转后重拉当前窗口三件套——预取 effect 的 deps 里
    // 没有 dayKey，不会自己重跑，不在这里拉的话邻居会一直停在 emptyValue。
    useEffect(() => {
      const t = setInterval(() => {
        if (!invalidateOnRollover()) return;
        void fetchOne(currentOffset - 1);
        void fetchOne(currentOffset, currentOffset === 0);
        void fetchOne(currentOffset + 1);
      }, 30_000);
      return () => clearInterval(t);
    }, [currentOffset, fetchOne, invalidateOnRollover]);

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
      invalidateOnRollover();
      void fetchOne(currentOffset - 1);
      void fetchOne(currentOffset, currentOffset === 0);
      void fetchOne(currentOffset + 1);
    }, [currentOffset, fetchOne, categories, invalidateOnRollover]);

    useEffect(() => {
      if (currentOffset !== 0) return;
      const t = setInterval(() => {
        // 翻转后除了 0 还要补邻居——只重拉 0 的话"昨天"面板会停在 emptyValue
        if (invalidateOnRollover()) {
          void fetchOne(-1);
          void fetchOne(1);
        }
        // 不再无条件 delete(0)：交给 fetchOne 自身的 inFlight 守卫去重，
        // 避免同一 offset=0 并发多发、回来乱序覆盖（"柱子忽长忽短"的另一来源）。
        void fetchOne(0, true);
      }, pollInterval);
      return () => clearInterval(t);
    }, [currentOffset, fetchOne, invalidateOnRollover]);

    const get = useCallback(
      (offset: number): TData => cache.get(offset) ?? emptyValue,
      [cache],
    );

    return { get };
  };
}
