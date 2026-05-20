import {
  api,
  dtoToDaySummary,
  type AppUsage,
  type DaySummary,
} from "../api/hindsight";
import { createUsageCache } from "./createUsageCache";

export interface WeekData {
  days: DaySummary[];
  apps: AppUsage[];
}

const EMPTY_WEEK: WeekData = { days: [], apps: [] };

export const useWeekCache = createUsageCache<WeekData>({
  fetch: async (offset, deviceId) => {
    const [dayDtos, apps] = await Promise.all([
      api.getWeekDays(offset, deviceId),
      // 超额取 200：RankedList 默认前 10，用户点展开后看到全部。
      api.getWeekApps(offset, 200, deviceId),
    ]);
    return { days: dayDtos.map(dtoToDaySummary), apps };
  },
  emptyValue: EMPTY_WEEK,
  pollInterval: 60_000,
});
