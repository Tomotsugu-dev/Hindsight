import { invoke } from "@tauri-apps/api/core";

export interface HourSegment {
  categoryId: string;
  minutes: number;
}

export interface HourSlot {
  hour: number;
  segments: HourSegment[];
}

export interface AppUsage {
  process: string;
  categoryId: string;
  minutes: number;
}

export interface DaySummaryDto {
  date: string;
  segments: HourSegment[];
}

export interface DaySummary {
  date: Date;
  segments: HourSegment[];
}

export function dtoToDaySummary(dto: DaySummaryDto): DaySummary {
  const [y, m, d] = dto.date.split("-").map((s) => parseInt(s, 10));
  return {
    date: new Date(y, m - 1, d),
    segments: dto.segments,
  };
}

export interface Category {
  id: string;
  name: string;
  color: string;
  icon: string;
  builtin: boolean;
  apps: string[];
}

export interface CategoryInput {
  name: string;
  color: string;
  icon: string;
}

export interface CategoryPatch {
  name?: string;
  color?: string;
  icon?: string;
}

export interface UnclassifiedApp {
  processName: string;
  minutes: number;
  lastSeenAt: string;
}

export interface CaptureStatus {
  running: boolean;
  todayCount: number;
  lastCaptureAt: string | null;
  lastError: string | null;
}

export const api = {
  getDayHours: (dayOffset: number) =>
    invoke<HourSlot[]>("get_day_hours", { dayOffset }),
  getDayApps: (dayOffset: number, limit?: number) =>
    invoke<AppUsage[]>("get_day_apps", { dayOffset, limit }),
  getWeekDays: (weekOffset: number) =>
    invoke<DaySummaryDto[]>("get_week_days", { weekOffset }),
  getWeekApps: (weekOffset: number, limit?: number) =>
    invoke<AppUsage[]>("get_week_apps", { weekOffset, limit }),
  getMonthDays: (monthOffset: number) =>
    invoke<DaySummaryDto[]>("get_month_days", { monthOffset }),
  getMonthApps: (monthOffset: number, limit?: number) =>
    invoke<AppUsage[]>("get_month_apps", { monthOffset, limit }),
  listCategories: () => invoke<Category[]>("list_categories"),
  createCategory: (input: CategoryInput) =>
    invoke<Category>("create_category", { input }),
  updateCategory: (id: string, patch: CategoryPatch) =>
    invoke<void>("update_category", { id, patch }),
  deleteCategory: (id: string) => invoke<void>("delete_category", { id }),
  assignApp: (processName: string, categoryId: string) =>
    invoke<void>("assign_app_to_category", { processName, categoryId }),
  unassignApp: (processName: string) =>
    invoke<void>("unassign_app", { processName }),
  listUnclassifiedApps: (daysBack?: number) =>
    invoke<UnclassifiedApp[]>("list_unclassified_apps", { daysBack }),
  startCapture: () => invoke<void>("start_capture"),
  stopCapture: () => invoke<void>("stop_capture"),
  getCaptureStatus: () => invoke<CaptureStatus>("get_capture_status"),
  getAppIcon: (processName: string) =>
    invoke<string | null>("get_app_icon", { processName }),
};
