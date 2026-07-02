// 给 Today 页"点小时柱子→筛选 apps 排行"用：lazy fetch 选中的小时 apps，
// 已经看过的 (offset, hour, deviceId) 在内存里短暂缓存，避免来回点同一根
// 柱子时反复发请求。dayOffset / deviceId 切换时缓存自动失效。

import { useEffect, useRef, useState } from "react";
import { api, type AppUsage } from "../api/hindsight";

interface CacheKey {
  offset: number;
  hour: number;
  deviceId: string | undefined;
}

const MAX_CACHE = 24; // 一天 24 小时够用
/** 今天(offset=0)的数据一直在涨——柱子/头部总时长 30s 轮询会长，明细卡着首拍
 *  不动会自相矛盾。今天的缓存 30s 过期重拉；历史日不变，会话内长存。 */
const TODAY_TTL_MS = 30_000;
type Cache = Map<string, { apps: AppUsage[]; at: number }>;

function cacheKey(k: CacheKey): string {
  return `${k.offset}|${k.hour}|${k.deviceId ?? ""}`;
}

interface State {
  apps: AppUsage[] | null;
  loading: boolean;
}

export function useHourApps(
  dayOffset: number,
  hour: number | null,
  deviceId?: string,
): State {
  const cacheRef = useRef<Cache>(new Map());
  const [state, setState] = useState<State>({ apps: null, loading: false });

  // dayOffset / deviceId 改变 → 清缓存（之前小时数据不再有意义）
  const lastScopeRef = useRef<{ offset: number; deviceId: string | undefined }>({
    offset: dayOffset,
    deviceId,
  });
  if (
    lastScopeRef.current.offset !== dayOffset ||
    lastScopeRef.current.deviceId !== deviceId
  ) {
    cacheRef.current.clear();
    lastScopeRef.current = { offset: dayOffset, deviceId };
  }

  useEffect(() => {
    if (hour === null) {
      setState({ apps: null, loading: false });
      return;
    }
    const key = cacheKey({ offset: dayOffset, hour, deviceId });
    const cached = cacheRef.current.get(key);
    if (cached && !(dayOffset === 0 && Date.now() - cached.at > TODAY_TTL_MS)) {
      setState({ apps: cached.apps, loading: false });
      return;
    }

    let cancelled = false;
    setState((prev) => ({ apps: prev.apps, loading: true }));
    api
      .getHourApps(dayOffset, hour, 10, deviceId)
      .then((apps) => {
        if (cancelled) return;
        const cache = cacheRef.current;
        cache.set(key, { apps, at: Date.now() });
        // 简单 LRU：超过 MAX_CACHE 删最早条目
        if (cache.size > MAX_CACHE) {
          const oldest = cache.keys().next().value;
          if (oldest !== undefined) cache.delete(oldest);
        }
        setState({ apps, loading: false });
      })
      .catch(() => {
        if (cancelled) return;
        setState({ apps: [], loading: false });
      });
    return () => {
      cancelled = true;
    };
  }, [dayOffset, hour, deviceId]);

  return state;
}
