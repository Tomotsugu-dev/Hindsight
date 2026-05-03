import { useCallback, useEffect, useRef, useState } from "react";
import { api, type AppUsage, type HourSlot } from "../api/hindsight";
import { useCategories } from "../state/categories";

interface DayData {
  hours: HourSlot[];
  apps: AppUsage[];
}

const EMPTY_HOURS: HourSlot[] = Array.from({ length: 24 }, (_, h) => ({
  hour: h,
  segments: [],
}));

const EMPTY_DAY: DayData = { hours: EMPTY_HOURS, apps: [] };

/**
 * `deviceId === undefined` 表示"全部设备聚合"；具体 UUID 表示只看该设备。
 * 切换 deviceId / 分类数据变更时清空缓存重新拉取。
 */
export function useDayCache(currentOffset: number, deviceId?: string) {
  const { categories } = useCategories();
  const [cache, setCache] = useState<Map<number, DayData>>(new Map());
  const inFlightRef = useRef<Set<number>>(new Set());

  const fetchOne = useCallback(
    async (offset: number) => {
      if (offset > 0) return;
      if (inFlightRef.current.has(offset)) return;
      inFlightRef.current.add(offset);
      try {
        const [hours, apps] = await Promise.all([
          api.getDayHours(offset, deviceId),
          api.getDayApps(offset, 10, deviceId),
        ]);
        setCache((prev) => {
          const next = new Map(prev);
          next.set(offset, { hours, apps });
          return next;
        });
      } catch {
        // 查询失败静默
      } finally {
        inFlightRef.current.delete(offset);
      }
    },
    [deviceId],
  );

  // 切设备 / categories 引用变化（CategoriesProvider 每次 refresh 后都换新数组）→
  // 清空缓存重新拉。这样应用分类页里指派 / 配对操作完，Today / Week / Month 立刻反映。
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
    }, 30_000);
    return () => clearInterval(t);
  }, [currentOffset, fetchOne]);

  const get = useCallback(
    (offset: number): DayData => cache.get(offset) ?? EMPTY_DAY,
    [cache],
  );

  return { get };
}
