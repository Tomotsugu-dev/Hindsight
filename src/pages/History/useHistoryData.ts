import { useEffect, useState } from "react";
import {
  api,
  type AppDetail,
  type AppUsage,
  type DaySummaryDto,
} from "../../api/hindsight";
import { logError } from "../../lib/logger";

interface Loaded<T> {
  /** 这一组参数的结果；还没拉到时为 null */
  data: T | null;
  loading: boolean;
  /** 最近一次拉到的结果，不管是哪组参数的；换范围时先拿它顶着，列表不闪 */
  latest: T | null;
}

/**
 * 参数一变就重新拉，只保留最后一组参数的结果；`args = null` 时不拉。
 * `load` 必须是模块级函数：参数靠 JSON 序列化比较，函数本身不进依赖。
 */
function useLoad<A extends unknown[], T>(
  args: A | null,
  load: (...args: A) => Promise<T>,
): Loaded<T> {
  const key = args === null ? null : JSON.stringify(args);
  const [state, setState] = useState<{ key: string | null; data: T | null }>({
    key: null,
    data: null,
  });

  useEffect(() => {
    if (key === null) return;
    let cancelled = false;
    load(...(JSON.parse(key) as A)).then(
      (data) => {
        if (!cancelled) setState({ key, data });
      },
      (e: unknown) => {
        logError("history.load", e);
        if (!cancelled) setState({ key, data: null });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [key, load]);

  // 结果还是上一组参数的：当作没拉到，免得换了应用还显示上一个应用的标题
  const fresh = key !== null && state.key === key;
  return {
    data: fresh ? state.data : null,
    loading: key !== null && !fresh,
    latest: state.data,
  };
}

async function loadEarliest(): Promise<string | null> {
  return api.earliestActivityDate();
}

/** 第一条记录的日期；空库为 null，还在查为 undefined。 */
export function useEarliestDate(): string | null | undefined {
  const { data, loading } = useLoad([], loadEarliest);
  return loading ? undefined : data;
}

async function loadDays(
  from: string,
  to: string,
  deviceId: string | null,
): Promise<DaySummaryDto[]> {
  return api.getRangeCategoryTime(from, to, deviceId ?? undefined);
}

/** from..to 每天各分类的时长；from 为 null 时不拉。 */
export function useRangeDays(from: string | null, to: string, deviceId: string | undefined) {
  return useLoad(from === null ? null : [from, to, deviceId ?? null], loadDays);
}

async function loadApps(
  from: string,
  to: string,
  deviceId: string | null,
): Promise<AppUsage[]> {
  return api.getRangeApps(from, to, deviceId ?? undefined);
}

/** from..to 每个应用的总时长；from 为 null 时不拉。 */
export function useRangeApps(
  from: string | null,
  to: string | null,
  deviceId: string | undefined,
) {
  return useLoad(
    from === null || to === null ? null : [from, to, deviceId ?? null],
    loadApps,
  );
}

async function loadAppDetail(
  from: string,
  to: string,
  groupId: string,
  deviceId: string | null,
): Promise<AppDetail> {
  return api.getAppRangeDetail(from, to, groupId, deviceId ?? undefined);
}

/** 一个应用在 from..to 每天的时长和窗口标题；任一参数为 null 时不拉。 */
export function useAppRangeDetail(
  groupId: string | null,
  from: string | null,
  to: string | null,
  deviceId: string | undefined,
) {
  return useLoad(
    groupId === null || from === null || to === null
      ? null
      : [from, to, groupId, deviceId ?? null],
    loadAppDetail,
  );
}
