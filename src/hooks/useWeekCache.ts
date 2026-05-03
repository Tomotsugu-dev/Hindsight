import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  dtoToDaySummary,
  type AppUsage,
  type DaySummary,
} from "../api/hindsight";

interface WeekData {
  days: DaySummary[];
  apps: AppUsage[];
}

const EMPTY_WEEK: WeekData = { days: [], apps: [] };

export function useWeekCache(currentOffset: number, deviceId?: string) {
  const [cache, setCache] = useState<Map<number, WeekData>>(new Map());
  const inFlightRef = useRef<Set<number>>(new Set());

  const fetchOne = useCallback(
    async (offset: number) => {
      if (offset > 0) return;
      if (inFlightRef.current.has(offset)) return;
      inFlightRef.current.add(offset);
      try {
        const [dayDtos, apps] = await Promise.all([
          api.getWeekDays(offset, deviceId),
          api.getWeekApps(offset, 10, deviceId),
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

  useEffect(() => {
    setCache(new Map());
    inFlightRef.current.clear();
  }, [deviceId]);

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
    (offset: number): WeekData => cache.get(offset) ?? EMPTY_WEEK,
    [cache],
  );

  return { get };
}
