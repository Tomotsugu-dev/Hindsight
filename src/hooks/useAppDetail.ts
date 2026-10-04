// 给「点应用 → 详情抽屉」用：按 scope(day/week/month) lazy fetch 选中 app 的聚合详情
// （时间柱 + 窗口标题用时）。缓存按 (scope, offset, groupId, deviceId) 全键，LRU 淘汰。

import { useEffect, useRef, useState } from "react";
import { api, type AppDetail } from "../api/hindsight";

export type DetailScope = "day" | "week" | "month";

const MAX_CACHE = 32;
/** 当前期(offset=0)的数据一直在涨——同 useHourApps 的 TTL 语义，30s 过期重拉。
 *  不加的话"今天"的详情抽屉整个会话都停在首拍快照，和旁边 30s 轮询的柱子自相矛盾。 */
const TODAY_TTL_MS = 30_000;

function cacheKey(
  scope: DetailScope,
  offset: number,
  groupId: string,
  deviceId: string | undefined,
): string {
  return `${scope}|${offset}|${groupId}|${deviceId ?? ""}`;
}

function fetchDetail(
  scope: DetailScope,
  offset: number,
  groupId: string,
  deviceId?: string,
): Promise<AppDetail> {
  if (scope === "week") return api.getAppWeekDetail(offset, groupId, deviceId);
  if (scope === "month")
    return api.getAppMonthDetail(offset, groupId, deviceId);
  return api.getAppDayDetail(offset, groupId, deviceId);
}

interface State {
  detail: AppDetail | null;
  loading: boolean;
}

interface Stored extends State {
  /** 这份结果是哪组参数的；参数变了、effect 还没跑的那一次渲染里拿它判断结果是不是旧的 */
  key: string | null;
}

/** `groupId === null` = 没选 app（抽屉关着）→ 不请求、返回 null。 */
export function useAppDetail(
  scope: DetailScope,
  offset: number,
  groupId: string | null,
  deviceId?: string,
): State {
  const cacheRef = useRef<Map<string, { detail: AppDetail; at: number }>>(new Map());
  const [state, setState] = useState<Stored>({ detail: null, loading: false, key: null });
  const currentKey = groupId === null ? null : cacheKey(scope, offset, groupId, deviceId);

  // 跨午夜失效：缓存键是相对 offset，后端按查询时刻的 now 解析——过夜后同一个
  // key 指向的日历日变了，不清会把昨天的详情当今天的展示（同 createUsageCache）。
  const dayKeyRef = useRef(new Date().toDateString());
  const dayKey = new Date().toDateString();
  if (dayKeyRef.current !== dayKey) {
    cacheRef.current.clear();
    dayKeyRef.current = dayKey;
  }

  useEffect(() => {
    if (groupId === null) {
      setState({ detail: null, loading: false, key: null });
      return;
    }
    const key = cacheKey(scope, offset, groupId, deviceId);
    const cached = cacheRef.current.get(key);
    if (cached && !(offset === 0 && Date.now() - cached.at > TODAY_TTL_MS)) {
      setState({ detail: cached.detail, loading: false, key });
      return;
    }

    let cancelled = false;
    setState({ detail: null, loading: true, key });
    fetchDetail(scope, offset, groupId, deviceId)
      .then((detail) => {
        if (cancelled) return;
        const cache = cacheRef.current;
        cache.set(key, { detail, at: Date.now() });
        if (cache.size > MAX_CACHE) {
          const oldest = cache.keys().next().value;
          if (oldest !== undefined) cache.delete(oldest);
        }
        setState({ detail, loading: false, key });
      })
      .catch(() => {
        if (cancelled) return;
        setState({ detail: { buckets: [], titles: [], isBrowser: false }, loading: false, key });
      });
    return () => {
      cancelled = true;
    };
  }, [scope, offset, groupId, deviceId]);

  // 换了应用或范围、effect 还没跑：手上的是上一组参数的结果，不能拿出去用
  if (state.key !== currentKey) {
    return { detail: null, loading: currentKey !== null };
  }
  return { detail: state.detail, loading: state.loading };
}
