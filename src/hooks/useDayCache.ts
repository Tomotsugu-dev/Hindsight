import { useCallback, useEffect, useRef, useState } from "react";
import { api, type AppUsage, type HourSlot } from "../api/hindsight";

interface DayData {
  hours: HourSlot[];
  apps: AppUsage[];
}

const EMPTY_HOURS: HourSlot[] = Array.from({ length: 24 }, (_, h) => ({
  hour: h,
  segments: [],
}));

const EMPTY_DAY: DayData = { hours: EMPTY_HOURS, apps: [] };

export function useDayCache(currentOffset: number) {
  const [cache, setCache] = useState<Map<number, DayData>>(new Map());
  const inFlightRef = useRef<Set<number>>(new Set());

  const fetchOne = useCallback(async (offset: number) => {
    if (offset > 0) return;
    if (inFlightRef.current.has(offset)) return;
    inFlightRef.current.add(offset);
    try {
      const [hours, apps] = await Promise.all([
        api.getDayHours(offset),
        api.getDayApps(offset, 10),
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
  }, []);

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
