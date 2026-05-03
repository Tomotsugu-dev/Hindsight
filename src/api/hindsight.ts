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

export interface TimeRange {
  start: string;
  end: string;
}

export interface Settings {
  captureEnabled: boolean;
  captureIntervalSeconds: number;
  screenshotPath: string;
  workHoursEnabled: boolean;
  workRanges: TimeRange[];
  autoStart: boolean;
  showWindowOnAutoStart: boolean;
  retentionDays: number;
  /** Google Cloud Console 创建的 Desktop App OAuth 凭证（Drive 同步用） */
  googleClientId: string;
  googleClientSecret: string;
}

export type SettingsPatch = Partial<Settings>;

export interface StorageInfo {
  dbBytes: number;
  screenshotsBytes: number;
  dbPath: string;
  screenshotsPath: string;
}

export interface DeviceRow {
  deviceId: string;
  displayName: string;
  color: string;
  icon: string;
  os: string | null;
  lastSeenAt: string | null;
  isSelf: boolean;
}

export interface AuthState {
  signedIn: boolean;
  uid: string | null;
  email: string | null;
  /** OAuth 凭证是否齐全（决定登录按钮是否可点） */
  configured: boolean;
}

export interface SyncStatus {
  running: boolean;
  lastPushedAt: string | null;
  lastPulledAt: string | null;
  lastError: string | null;
  pending: number;
  deadLetter: number;
}

export const api = {
  getDayHours: (dayOffset: number, deviceId?: string) =>
    invoke<HourSlot[]>("get_day_hours", { dayOffset, deviceId }),
  getDayApps: (dayOffset: number, limit?: number, deviceId?: string) =>
    invoke<AppUsage[]>("get_day_apps", { dayOffset, limit, deviceId }),
  getWeekDays: (weekOffset: number, deviceId?: string) =>
    invoke<DaySummaryDto[]>("get_week_days", { weekOffset, deviceId }),
  getWeekApps: (weekOffset: number, limit?: number, deviceId?: string) =>
    invoke<AppUsage[]>("get_week_apps", { weekOffset, limit, deviceId }),
  getMonthDays: (monthOffset: number, deviceId?: string) =>
    invoke<DaySummaryDto[]>("get_month_days", { monthOffset, deviceId }),
  getMonthApps: (monthOffset: number, limit?: number, deviceId?: string) =>
    invoke<AppUsage[]>("get_month_apps", { monthOffset, limit, deviceId }),
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
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: SettingsPatch) =>
    invoke<Settings>("update_settings", { patch }),
  getStorageInfo: () => invoke<StorageInfo>("get_storage_info"),
  purgeActivities: () => invoke<void>("purge_activities"),
  purgeScreenshots: () => invoke<void>("purge_screenshots"),
  openScreenshotsDir: () => invoke<void>("open_screenshots_dir"),
  getDataRoot: () => invoke<string>("get_data_root"),
  setDataRoot: (path: string) => invoke<void>("set_data_root", { path }),
  listDevices: () => invoke<DeviceRow[]>("list_devices"),
  updateSelfDevice: (
    name?: string,
    color?: string,
    icon?: string,
  ) => invoke<DeviceRow>("update_self_device", { name, color, icon }),
  authStatus: () => invoke<AuthState>("auth_status"),
  signInWithGoogle: () => invoke<AuthState>("sign_in_with_google"),
  signOut: () => invoke<void>("sign_out"),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncNow: () => invoke<void>("sync_now"),
};
