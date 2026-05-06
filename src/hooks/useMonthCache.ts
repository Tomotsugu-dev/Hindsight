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
      api.getMonthApps(offset, 10, deviceId),
    ]);
    return { days: dayDtos.map(dtoToDaySummary), apps };
  },
  emptyValue: EMPTY_MONTH,
  pollInterval: 60_000,
});
