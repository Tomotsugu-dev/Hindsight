import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  dtoToDaySummary,
  type AppUsage,
  type DaySummary,
} from "../api/hindsight";
import { useCategories } from "../state/categories";

interface MonthData {
  days: DaySummary[];
  apps: AppUsage[];
}

const EMPTY_MONTH: MonthData = { days: [], apps: [] };

export function useMonthCache(currentOffset: number, deviceId?: string) {
  const { categories } = useCategories();
  const [cache, setCache] = useState<Map<number, MonthData>>(new Map());
  const inFlightRef = useRef<Set<number>>(new Set());

  const fetchOne = useCallback(
    async (offset: number) => {
      if (offset > 0) return;
      if (inFlightRef.current.has(offset)) return;
      inFlightRef.current.add(offset);
      try {
        const [dayDtos, apps] = await Promise.all([
          api.getMonthDays(offset, deviceId),
          api.getMonthApps(offset, 10, deviceId),
        ]);
        const days = dayDtos.map(dtoToDaySummary);
        setCache((prev) => {
          const next = new Map(prev);
          next.set(offset, { days, apps });
          return next;
        });
      } catch {
        /* ignore */
      } finally {
        inFlightRef.current.delete(offset);
      }
    },
    [deviceId],
  );

  // 切设备 / categories 引用变化（CategoriesProvider 每次 refresh 后都换新数组）→
  // 清空缓存重新拉，让分类页里的指派 / 配对操作立刻反映到本月数据上。
  useEffect(() => {
    setCache(new Map());
    inFlightRef.current.clear();
  }, [deviceId, categories]);

  useEffect(() => {
    fetchOne(currentOffset - 1);
    fetchOne(currentOffset);
    fetchOne(currentOffset + 1);
  }, [currentOffset, fetchOne]);

  useEffect(() => {
    if (currentOffset !== 0) return;
    const t = setInterval(() => {
      inFlightRef.current.delete(0);
      fetchOne(0);
    }, 60_000);
    return () => clearInterval(t);
  }, [currentOffset, fetchOne]);

  const get = useCallback(
    (offset: number): MonthData => cache.get(offset) ?? EMPTY_MONTH,
    [cache],
  );

  return { get };
}
