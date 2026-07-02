// Week / Month 页"点某一天柱子→筛选 apps 排行"用：lazy fetch 选中日期的 top apps，
// 复用 backend 既有的 get_day_apps（按相对今天的 dayOffset 查）。
// dayOffset === null → 不请求；dayOffset / deviceId 改变时取新值，旧的丢弃。

import { useEffect, useRef, useState } from "react";
import { api, type AppUsage } from "../api/hindsight";

const MAX_CACHE = 16; // Week 7 + Month ~30 + 余量
/** 今天(offset=0)仍在累积——同 useHourApps 的 TTL 语义，30s 过期重拉。 */
const TODAY_TTL_MS = 30_000;

interface State {
  apps: AppUsage[] | null;
  loading: boolean;
}

function cacheKey(offset: number, deviceId: string | undefined): string {
  return `${offset}|${deviceId ?? ""}`;
}

export function useSelectedDayApps(
  dayOffset: number | null,
  deviceId?: string,
): State {
  const cacheRef = useRef<Map<string, { apps: AppUsage[]; at: number }>>(new Map());
  const [state, setState] = useState<State>({ apps: null, loading: false });

  // deviceId 切换 → 缓存全部失效
  const lastDeviceRef = useRef<string | undefined>(deviceId);
  if (lastDeviceRef.current !== deviceId) {
    cacheRef.current.clear();
    lastDeviceRef.current = deviceId;
  }

  // 跨午夜失效：键是相对 offset、后端按查询时刻解析——过夜后 offset=-2 从周一
  // 变成周二，不清就是"标签周二、数据周一"（同 createUsageCache 的翻转语义）
  const dayKeyRef = useRef(new Date().toDateString());
  const dayKey = new Date().toDateString();
  if (dayKeyRef.current !== dayKey) {
    cacheRef.current.clear();
    dayKeyRef.current = dayKey;
  }

  useEffect(() => {
    if (dayOffset === null) {
      setState({ apps: null, loading: false });
      return;
    }
    const key = cacheKey(dayOffset, deviceId);
    const cached = cacheRef.current.get(key);
    if (cached && !(dayOffset === 0 && Date.now() - cached.at > TODAY_TTL_MS)) {
      setState({ apps: cached.apps, loading: false });
      return;
    }

    let cancelled = false;
    setState((prev) => ({ apps: prev.apps, loading: true }));
    api
      .getDayApps(dayOffset, 10, deviceId)
      .then((apps) => {
        if (cancelled) return;
        const cache = cacheRef.current;
        cache.set(key, { apps, at: Date.now() });
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
  }, [dayOffset, deviceId]);

  return state;
}
