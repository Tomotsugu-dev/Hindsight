// 按 scope(day/week/month) lazy fetch 选中 app 的聚合详情（时间柱 + 窗口标题用时）。
// 两处用：悬停排行行（useAppFocus）和点开应用的详情抽屉。缓存放在模块里，两处共用，
// 按 (scope, offset, groupId, deviceId) 全键，超过上限先删最早放进去的。

import { useEffect, useState } from "react";
import { api, type AppDetail } from "../api/hindsight";

export type DetailScope = "day" | "week" | "month";

const MAX_CACHE = 32;
/** 当前期(offset=0)的数据一直在涨——同 useHourApps 的 TTL 语义，30s 过期重拉。
 *  不加的话"今天"的详情抽屉整个会话都停在首拍快照，和旁边 30s 轮询的柱子自相矛盾。 */
const TODAY_TTL_MS = 30_000;

/** 悬停和详情抽屉共用：先悬停再点开，不用再查一遍 */
const cache = new Map<string, { detail: AppDetail; at: number }>();
/** 还没回来的请求：悬停的那次还在查时点开抽屉，等同一个结果，不再发一次 */
const inFlight = new Map<string, Promise<AppDetail>>();
/** 跨午夜失效：缓存键是相对 offset，后端按查询时刻的 now 解析——过夜后同一个
 *  key 指向的日历日变了，不清会把昨天的详情当今天的展示（同 createUsageCache）。 */
let cacheDay = new Date().toDateString();

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
  if (scope === "month") return api.getAppMonthDetail(offset, groupId, deviceId);
  return api.getAppDayDetail(offset, groupId, deviceId);
}

function clearIfNewDay(): void {
  const today = new Date().toDateString();
  if (cacheDay !== today) {
    cache.clear();
    cacheDay = today;
  }
}

/** 缓存里还能用的详情；没有，或者当前期的已经过了 30 秒，返回 null */
export function cachedAppDetail(
  scope: DetailScope,
  offset: number,
  groupId: string,
  deviceId: string | undefined,
): AppDetail | null {
  clearIfNewDay();
  const hit = cache.get(cacheKey(scope, offset, groupId, deviceId));
  if (!hit || (offset === 0 && Date.now() - hit.at > TODAY_TTL_MS)) return null;
  return hit.detail;
}

/** 拿详情：缓存里有就用，同样的请求还在查就等它，都没有才发请求。失败的不缓存 */
export function loadAppDetail(
  scope: DetailScope,
  offset: number,
  groupId: string,
  deviceId: string | undefined,
): Promise<AppDetail> {
  const cached = cachedAppDetail(scope, offset, groupId, deviceId);
  if (cached) return Promise.resolve(cached);
  const key = cacheKey(scope, offset, groupId, deviceId);
  const pending = inFlight.get(key);
  if (pending) return pending;
  const request = fetchDetail(scope, offset, groupId, deviceId)
    .then((detail) => {
      cache.set(key, { detail, at: Date.now() });
      if (cache.size > MAX_CACHE) {
        const oldest = cache.keys().next().value;
        if (oldest !== undefined) cache.delete(oldest);
      }
      return detail;
    })
    .finally(() => inFlight.delete(key));
  inFlight.set(key, request);
  return request;
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
  const [state, setState] = useState<Stored>({ detail: null, loading: false, key: null });
  const currentKey = groupId === null ? null : cacheKey(scope, offset, groupId, deviceId);

  useEffect(() => {
    if (groupId === null) {
      setState({ detail: null, loading: false, key: null });
      return;
    }
    const key = cacheKey(scope, offset, groupId, deviceId);
    const cached = cachedAppDetail(scope, offset, groupId, deviceId);
    if (cached) {
      setState({ detail: cached, loading: false, key });
      return;
    }

    let cancelled = false;
    setState({ detail: null, loading: true, key });
    loadAppDetail(scope, offset, groupId, deviceId)
      .then((detail) => {
        if (!cancelled) setState({ detail, loading: false, key });
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
