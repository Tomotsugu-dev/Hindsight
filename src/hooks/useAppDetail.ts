// 按 scope(day/week/month) lazy fetch 选中 app 的详情，分两份查、两份缓存：
// - 每天（日统计是每小时）的用时：悬停排行行（useAppFocus）只要这份，算峰值和日均；
// - 窗口标题用时：只有点开详情抽屉才查。
// 抽屉的时间柱直接用悬停时查好的那份。缓存放在模块里，两处共用，
// 按 (scope, offset, groupId, deviceId) 全键，超过上限先删最早放进去的。

import { useEffect, useState } from "react";
import {
  api,
  type AppDetail,
  type AppTitles,
  type DetailBucket,
  type DetailScope,
} from "../api/hindsight";

export type { DetailScope } from "../api/hindsight";

const MAX_CACHE = 32;
/** 当前期(offset=0)的数据一直在涨——同 useHourApps 的 TTL 语义，30s 过期重拉。
 *  不加的话"今天"的详情抽屉整个会话都停在首拍快照，和旁边 30s 轮询的柱子自相矛盾。 */
const TODAY_TTL_MS = 30_000;

type Lookup<T> = (
  scope: DetailScope,
  offset: number,
  groupId: string,
  deviceId: string | undefined,
) => T;

function cacheKey(
  scope: DetailScope,
  offset: number,
  groupId: string,
  deviceId: string | undefined,
): string {
  return `${scope}|${offset}|${groupId}|${deviceId ?? ""}`;
}

/** 一份按全键存的缓存：查过的直接拿，同样的请求还在查就等它，都没有才发请求。失败的不缓存 */
function createDetailCache<T>(fetch: Lookup<Promise<T>>) {
  /** 悬停和详情抽屉共用：先悬停再点开，不用再查一遍 */
  const cache = new Map<string, { value: T; at: number }>();
  /** 还没回来的请求：悬停的那次还在查时点开抽屉，等同一个结果，不再发一次 */
  const inFlight = new Map<string, Promise<T>>();
  /** 跨午夜失效：缓存键是相对 offset，后端按查询时刻的 now 解析——过夜后同一个
   *  key 指向的日历日变了，不清会把昨天的详情当今天的展示（同 createUsageCache）。 */
  let cacheDay = new Date().toDateString();

  function clearIfNewDay(): void {
    const today = new Date().toDateString();
    if (cacheDay !== today) {
      cache.clear();
      cacheDay = today;
    }
  }

  /** 缓存里还能用的；没有，或者当前期的已经过了 30 秒，返回 null */
  const cached: Lookup<T | null> = (scope, offset, groupId, deviceId) => {
    clearIfNewDay();
    const hit = cache.get(cacheKey(scope, offset, groupId, deviceId));
    if (!hit || (offset === 0 && Date.now() - hit.at > TODAY_TTL_MS)) return null;
    return hit.value;
  };

  const load: Lookup<Promise<T>> = (scope, offset, groupId, deviceId) => {
    const hit = cached(scope, offset, groupId, deviceId);
    if (hit) return Promise.resolve(hit);
    const key = cacheKey(scope, offset, groupId, deviceId);
    const pending = inFlight.get(key);
    if (pending) return pending;
    const request = fetch(scope, offset, groupId, deviceId)
      .then((value) => {
        cache.set(key, { value, at: Date.now() });
        if (cache.size > MAX_CACHE) {
          const oldest = cache.keys().next().value;
          if (oldest !== undefined) cache.delete(oldest);
        }
        return value;
      })
      .finally(() => inFlight.delete(key));
    inFlight.set(key, request);
    return request;
  };

  return { cached, load };
}

const bucketCache = createDetailCache<DetailBucket[]>((scope, offset, groupId, deviceId) =>
  api.getAppBuckets(scope, offset, groupId, deviceId),
);
const titleCache = createDetailCache<AppTitles>((scope, offset, groupId, deviceId) =>
  api.getAppTitles(scope, offset, groupId, deviceId),
);

export const cachedAppBuckets = bucketCache.cached;
export const loadAppBuckets = bucketCache.load;

const NO_BUCKETS: DetailBucket[] = [];
const NO_TITLES: AppTitles = { titles: [], isBrowser: false };
const EMPTY_DETAIL: AppDetail = { buckets: NO_BUCKETS, ...NO_TITLES };

/** 抽屉要的完整详情：两份都在缓存里才算有 */
export const cachedAppDetail: Lookup<AppDetail | null> = (scope, offset, groupId, deviceId) => {
  const buckets = bucketCache.cached(scope, offset, groupId, deviceId);
  const titles = titleCache.cached(scope, offset, groupId, deviceId);
  return buckets && titles ? { buckets, ...titles } : null;
};

/** 抽屉要的完整详情：两份各拿各的，哪份查失败就当作空的（失败的不缓存，下次重查） */
export const loadAppDetail: Lookup<Promise<AppDetail>> = (scope, offset, groupId, deviceId) =>
  Promise.all([
    bucketCache.load(scope, offset, groupId, deviceId).catch(() => NO_BUCKETS),
    titleCache.load(scope, offset, groupId, deviceId).catch(() => NO_TITLES),
  ]).then(([buckets, titles]) => ({ buckets, ...titles }));

interface Loaded<T> {
  value: T | null;
  loading: boolean;
}

interface Stored<T> extends Loaded<T> {
  /** 这份结果是哪组参数的；参数变了、effect 还没跑的那一次渲染里拿它判断结果是不是旧的 */
  key: string | null;
}

/** 按参数从缓存里拿；`groupId === null` = 没选 app → 不请求、返回 null。请求失败时给 `empty` */
function useCached<T>(
  cached: Lookup<T | null>,
  load: Lookup<Promise<T>>,
  empty: T,
  scope: DetailScope,
  offset: number,
  groupId: string | null,
  deviceId: string | undefined,
): Loaded<T> {
  const [state, setState] = useState<Stored<T>>({ value: null, loading: false, key: null });
  const currentKey = groupId === null ? null : cacheKey(scope, offset, groupId, deviceId);

  useEffect(() => {
    if (groupId === null) {
      setState({ value: null, loading: false, key: null });
      return;
    }
    const key = cacheKey(scope, offset, groupId, deviceId);
    const hit = cached(scope, offset, groupId, deviceId);
    if (hit) {
      setState({ value: hit, loading: false, key });
      return;
    }

    let cancelled = false;
    setState({ value: null, loading: true, key });
    load(scope, offset, groupId, deviceId)
      .then((value) => {
        if (!cancelled) setState({ value, loading: false, key });
      })
      .catch(() => {
        if (!cancelled) setState({ value: empty, loading: false, key });
      });
    return () => {
      cancelled = true;
    };
  }, [cached, load, empty, scope, offset, groupId, deviceId]);

  // 换了应用或范围、effect 还没跑：手上的是上一组参数的结果，不能拿出去用
  if (state.key !== currentKey) {
    return { value: null, loading: currentKey !== null };
  }
  return { value: state.value, loading: state.loading };
}

/** 悬停用：一个应用每天（日统计是每小时）的用时。`groupId === null` = 没停在应用上 */
export function useAppBuckets(
  scope: DetailScope,
  offset: number,
  groupId: string | null,
  deviceId?: string,
): { buckets: DetailBucket[] | null; loading: boolean } {
  const { value, loading } = useCached(
    cachedAppBuckets,
    loadAppBuckets,
    NO_BUCKETS,
    scope,
    offset,
    groupId,
    deviceId,
  );
  return { buckets: value, loading };
}

/** 详情抽屉用：时间柱 + 窗口标题。`groupId === null` = 抽屉关着 */
export function useAppDetail(
  scope: DetailScope,
  offset: number,
  groupId: string | null,
  deviceId?: string,
): { detail: AppDetail | null; loading: boolean } {
  const { value, loading } = useCached(
    cachedAppDetail,
    loadAppDetail,
    EMPTY_DETAIL,
    scope,
    offset,
    groupId,
    deviceId,
  );
  return { detail: value, loading };
}
