import { api, type AppUsage, type HourSlot } from "../api/hindsight";
import { createUsageCache } from "./createUsageCache";

export interface DayData {
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
 *
 * 实现共享在 createUsageCache 工厂中——本文件仅声明 day 维度的 fetch 与空值。
 */
export const useDayCache = createUsageCache<DayData>({
  fetch: async (offset, deviceId) => {
    const [hours, apps] = await Promise.all([
      api.getDayHours(offset, deviceId),
      api.getDayApps(offset, 10, deviceId),
    ]);
    return { hours, apps };
  },
  emptyValue: EMPTY_DAY,
  pollInterval: 30_000,
});
