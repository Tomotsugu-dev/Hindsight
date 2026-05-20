import {
  api,
  dtoToDaySummary,
  type AppUsage,
  type DaySummary,
} from "../api/hindsight";
import { createUsageCache } from "./createUsageCache";

export interface MonthData {
  days: DaySummary[];
  apps: AppUsage[];
}

const EMPTY_MONTH: MonthData = { days: [], apps: [] };

export const useMonthCache = createUsageCache<MonthData>({
  fetch: async (offset, deviceId) => {
    const [dayDtos, apps] = await Promise.all([
      api.getMonthDays(offset, deviceId),
      // 超额取 200：RankedList 默认前 10，用户点展开后看到全部。
      api.getMonthApps(offset, 200, deviceId),
    ]);
    return { days: dayDtos.map(dtoToDaySummary), apps };
  },
  emptyValue: EMPTY_MONTH,
  pollInterval: 60_000,
});
