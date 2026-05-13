// API mock —— 替换 src/api/hindsight.ts 给 demo 用。
//
// 设计要点：
// - 类型 100% 复用主仓库（type-only import），保证 demo 编译跟主应用同步
// - 只读方法：从 fixtures 返数据（一些写操作改内存 + localStorage）
// - 长任务（AI 总结、模型下载）：用 setTimeout 模拟阶段性 emit 进度事件
// - 错误处理：尽量"成功"——demo 不应该让用户看到错误状态破坏体验

import type {
  AppGroup,
  AppUsage,
  AuthState,
  Category,
  CategoryInput,
  CategoryPatch,
  CaptureStatus,
  DaySummary,
  DaySummaryDto,
  DeviceRow,
  EngineStatus,
  HourSlot,
  ImageDescriptionRow,
  ModelEntry,
  PartialDownload,
  QuickDayPart,
  QuickDaySummary,
  QuickMonthSummary,
  QuickPeakDay,
  QuickUsageEntry,
  QuickWeekSummary,
  RecommendedModel,
  SegmentSummaryRow,
  Settings,
  SettingsPatch,
  StorageInfo,
  SyncStatus,
  TestAiEndpointResp,
  UnclassifiedApp,
  WeekPrecheckResp,
  AiOverrides,
} from "@app/api/hindsight";

// 重新导出原 api 模块里的 helper 函数（DTO → 内部类型转换）
// 主仓库代码会直接 import { dtoToDaySummary } from "@app/api/hindsight"
export function dtoToDaySummary(dto: DaySummaryDto): DaySummary {
  const [y, m, d] = dto.date.split("-").map((s) => parseInt(s, 10));
  return {
    date: new Date(y, m - 1, d),
    segments: dto.segments,
  };
}

// 重新导出原 api 里的事件常量
export const MODEL_DOWNLOAD_EVENT = "ai://model-download-progress";
export const ENGINE_DOWNLOAD_EVENT = "ai://engine-download-progress";
export const SUMMARY_PROGRESS_EVENT = "ai://summary-progress";

// ────────────────────────────────────────────
// 应用图标 —— 用 Simple Icons CDN 拿品牌 SVG
// CDN 格式: https://cdn.simpleicons.org/<slug>/<hex-color>
// (key 全小写匹配，main mock 把 processName.toLowerCase() 后查表)
// ────────────────────────────────────────────

// 本地 PNG 图标（用户提供原图）—— 真实品牌 logo，相对 demo base path
// 部署后 URL: https://hindsight.kyosweb.com/demo/icons/<name>.png
const LOCAL_ICON = (name: string) => `/demo/icons/${name}.png`;

const APP_ICON_URLS: Record<string, string> = {
  // 编程（VS Code + Cursor 用本地 PNG —— Simple Icons CDN 没收录 VS Code）
  "code.exe": LOCAL_ICON("vscode"),
  "cursor.exe": LOCAL_ICON("cursor"),
  "windowsterminal.exe": LOCAL_ICON("terminal"),
  "rustrover.exe": "https://cdn.simpleicons.org/rust/000000",
  // 浏览（Chrome 用本地 PNG，多彩 logo CDN 单色版没法还原）
  "chrome.exe": LOCAL_ICON("chrome"),
  "firefox.exe": "https://cdn.simpleicons.org/firefox/FF7139",
  "msedge.exe": LOCAL_ICON("edge"),
  // 沟通（Teams 用本地 PNG，多层紫色 logo 比 CDN 单色更准）
  "telegram.exe": "https://cdn.simpleicons.org/telegram/26A5E4",
  "teams.exe": LOCAL_ICON("teams"),
  "wechat.exe": "https://cdn.simpleicons.org/wechat/07C160",
  // 学习
  "obsidian.exe": "https://cdn.simpleicons.org/obsidian/7C3AED",
  "notion.exe": "https://cdn.simpleicons.org/notion/000000",
  // 娱乐
  "spotify.exe": "https://cdn.simpleicons.org/spotify/1DB954",
  "discord.exe": "https://cdn.simpleicons.org/discord/5865F2",
  "steam.exe": "https://cdn.simpleicons.org/steam/000000",
  // 系统（其他分类）
  "explorer.exe": LOCAL_ICON("explorer"),
  "systemsettings.exe": LOCAL_ICON("systemsettings"),
  // 杂项（unclassified 用）
  "githubdesktop.exe": "https://cdn.simpleicons.org/github/000000",
};

import { emit } from "./tauri/event";
import i18n from "@app/i18n";
import {
  mockCategories,
  mockSettings,
  mockDevices,
  mockAuthState,
  mockSyncStatus,
  mockEngineStatus,
  mockLocalModels,
  mockRecommendedModels,
  mockDailySegments,
  mockAppGroups,
  mockUnclassifiedApps,
  mockDayFor,
  dailySegmentsForLocale,
  userBriefForLocale,
} from "./fixtures";
import { persistence } from "./persistence";

// ────────────────────────────────────────────
// 内部状态 —— 从 fixtures 初始化 + localStorage 覆盖
// ────────────────────────────────────────────

interface MutableState {
  categories: Category[];
  settings: Settings;
  devices: DeviceRow[];
  authState: AuthState;
  syncStatus: SyncStatus;
  appGroups: AppGroup[];
  // 段总结按 source → date → segments
  daySummaries: Map<string, SegmentSummaryRow[]>;
}

function loadState(): MutableState {
  const saved = persistence.load();
  return {
    categories: (saved?.categories as Category[]) ?? structuredClone(mockCategories),
    settings: (saved?.settings as Settings) ?? structuredClone(mockSettings),
    devices: (saved?.selfDevice
      ? mockDevices.map((d) =>
          d.isSelf ? { ...d, ...(saved.selfDevice as Partial<DeviceRow>) } : d,
        )
      : structuredClone(mockDevices)) as DeviceRow[],
    authState: structuredClone(mockAuthState),
    syncStatus: structuredClone(mockSyncStatus),
    appGroups: (saved?.appGroups as AppGroup[]) ?? structuredClone(mockAppGroups),
    daySummaries: new Map([[`daily:${todayStr()}`, structuredClone(mockDailySegments)]]),
  };
}

const state = loadState();

function persist() {
  const selfDevice = state.devices.find((d) => d.isSelf);
  persistence.save({
    categories: state.categories,
    settings: state.settings,
    selfDevice,
    appGroups: state.appGroups,
  });
}

function todayStr(): string {
  return ymd(new Date());
}

function ymd(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function isoDateOffset(dayOffset: number): string {
  const d = new Date();
  d.setDate(d.getDate() + dayOffset);
  return ymd(d);
}


// ────────────────────────────────────────────
// api 对象 —— 跟原 hindsight.ts 同样接口
// ────────────────────────────────────────────

export const api = {
  // ─── 时段数据 ──────────────────────────────
  getDayHours: async (
    dayOffset: number,
    deviceId?: string,
  ): Promise<HourSlot[]> => {
    return mockDayFor(dayOffset, deviceId).hours;
  },
  getDayApps: async (
    dayOffset: number,
    limit?: number,
    deviceId?: string,
  ): Promise<AppUsage[]> => {
    const apps = mockDayFor(dayOffset, deviceId).apps;
    return limit ? apps.slice(0, limit) : apps;
  },
  getHourApps: async (
    dayOffset: number,
    hour: number,
    limit?: number,
    deviceId?: string,
  ): Promise<AppUsage[]> => {
    // 该小时的 apps —— 用全日 apps 按比例缩放代替（精确度对 demo 够用）
    const day = mockDayFor(dayOffset, deviceId);
    const slot = day.hours.find((h) => h.hour === hour);
    if (!slot || slot.segments.length === 0) return [];
    const hourCategoryIds = new Set(slot.segments.map((s) => s.categoryId));
    const hourApps = day.apps.filter((a) => hourCategoryIds.has(a.categoryId));
    // 估算 minute 比例（粗略，仅 demo 视觉用）
    const hourTotal = slot.segments.reduce((s, x) => s + x.minutes, 0);
    const dayTotal = day.apps.reduce((s, a) => s + a.minutes, 0);
    const scale = dayTotal > 0 ? hourTotal / dayTotal : 0;
    const scaled = hourApps.map((a) => ({
      ...a,
      minutes: Math.max(1, Math.round(a.minutes * scale * 2)),
    }));
    scaled.sort((a, b) => b.minutes - a.minutes);
    return limit ? scaled.slice(0, limit) : scaled;
  },

  getWeekDays: async (
    weekOffset: number,
    deviceId?: string,
  ): Promise<DaySummaryDto[]> => {
    // weekOffset=0 → 本周一到周日
    const today = new Date();
    const dow = today.getDay() || 7; // 周一=1 周日=7
    const monday = new Date(today);
    monday.setDate(today.getDate() - (dow - 1) + weekOffset * 7);
    const result: DaySummaryDto[] = [];
    for (let i = 0; i < 7; i++) {
      const d = new Date(monday);
      d.setDate(monday.getDate() + i);
      const offsetFromToday = Math.round(
        (d.getTime() - today.getTime()) / (24 * 3600 * 1000),
      );
      const day = mockDayFor(offsetFromToday, deviceId);
      // 全日聚合
      const segMap = new Map<string, number>();
      for (const slot of day.hours) {
        for (const seg of slot.segments) {
          segMap.set(seg.categoryId, (segMap.get(seg.categoryId) ?? 0) + seg.minutes);
        }
      }
      result.push({
        date: `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`,
        segments: Array.from(segMap.entries()).map(([categoryId, minutes]) => ({
          categoryId,
          minutes,
        })),
      });
    }
    return result;
  },
  getWeekApps: async (
    weekOffset: number,
    limit?: number,
    deviceId?: string,
  ): Promise<AppUsage[]> => {
    // 聚合 7 天 apps
    const today = new Date();
    const dow = today.getDay() || 7;
    const monday = new Date(today);
    monday.setDate(today.getDate() - (dow - 1) + weekOffset * 7);
    const map = new Map<string, AppUsage>();
    for (let i = 0; i < 7; i++) {
      const d = new Date(monday);
      d.setDate(monday.getDate() + i);
      const offset = Math.round((d.getTime() - today.getTime()) / (24 * 3600 * 1000));
      const day = mockDayFor(offset, deviceId);
      for (const a of day.apps) {
        const cur = map.get(a.process);
        if (cur) cur.minutes += a.minutes;
        else map.set(a.process, { ...a });
      }
    }
    const sorted = Array.from(map.values()).sort((a, b) => b.minutes - a.minutes);
    return limit ? sorted.slice(0, limit) : sorted;
  },

  getMonthDays: async (
    monthOffset: number,
    deviceId?: string,
  ): Promise<DaySummaryDto[]> => {
    // monthOffset=0 → 本月 1 日到月末
    const today = new Date();
    const target = new Date(today.getFullYear(), today.getMonth() + monthOffset, 1);
    const daysInMonth = new Date(
      target.getFullYear(),
      target.getMonth() + 1,
      0,
    ).getDate();
    const result: DaySummaryDto[] = [];
    for (let i = 0; i < daysInMonth; i++) {
      const d = new Date(target);
      d.setDate(1 + i);
      const offsetFromToday = Math.round(
        (d.getTime() - today.getTime()) / (24 * 3600 * 1000),
      );
      // 未来日期返空
      if (offsetFromToday > 0) {
        result.push({
          date: `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`,
          segments: [],
        });
        continue;
      }
      const day = mockDayFor(offsetFromToday, deviceId);
      const segMap = new Map<string, number>();
      for (const slot of day.hours) {
        for (const seg of slot.segments) {
          segMap.set(seg.categoryId, (segMap.get(seg.categoryId) ?? 0) + seg.minutes);
        }
      }
      result.push({
        date: `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`,
        segments: Array.from(segMap.entries()).map(([categoryId, minutes]) => ({
          categoryId,
          minutes,
        })),
      });
    }
    return result;
  },
  getMonthApps: async (
    monthOffset: number,
    limit?: number,
    deviceId?: string,
  ): Promise<AppUsage[]> => {
    const today = new Date();
    const target = new Date(today.getFullYear(), today.getMonth() + monthOffset, 1);
    const daysInMonth = new Date(
      target.getFullYear(),
      target.getMonth() + 1,
      0,
    ).getDate();
    const map = new Map<string, AppUsage>();
    for (let i = 0; i < daysInMonth; i++) {
      const d = new Date(target);
      d.setDate(1 + i);
      const offset = Math.round((d.getTime() - today.getTime()) / (24 * 3600 * 1000));
      if (offset > 0) continue;
      const day = mockDayFor(offset, deviceId);
      for (const a of day.apps) {
        const cur = map.get(a.process);
        if (cur) cur.minutes += a.minutes;
        else map.set(a.process, { ...a });
      }
    }
    const sorted = Array.from(map.values()).sort((a, b) => b.minutes - a.minutes);
    return limit ? sorted.slice(0, limit) : sorted;
  },

  // ─── 快速模板总结（无 LLM 依赖，纯聚合）─────
  getQuickDaySummary: async (
    dayOffset: number,
    deviceId?: string,
  ): Promise<QuickDaySummary> => {
    const day = mockDayFor(dayOffset, deviceId);
    const date = isoDateOffset(dayOffset);

    // 按小时聚合 → 找峰值小时 + 活跃小时数
    let peakHour = 0;
    let peakHourMinutes = 0;
    let activeHours = 0;
    let totalMinutes = 0;
    const hourTotals = new Array(24).fill(0);
    for (const slot of day.hours) {
      const hm = slot.segments.reduce((s, x) => s + x.minutes, 0);
      hourTotals[slot.hour] = hm;
      totalMinutes += hm;
      if (hm > 0) activeHours += 1;
      if (hm > peakHourMinutes) {
        peakHourMinutes = hm;
        peakHour = slot.hour;
      }
    }

    // 时段桶：night(0–5) / morning(6–11) / afternoon(12–17) / evening(18–23)
    const buckets: Record<string, number> = {
      night: 0,
      morning: 0,
      afternoon: 0,
      evening: 0,
    };
    for (let h = 0; h < 24; h++) {
      const bucket = h < 6 ? "night" : h < 12 ? "morning" : h < 18 ? "afternoon" : "evening";
      buckets[bucket] += hourTotals[h];
    }
    const dayParts: QuickDayPart[] = (["night", "morning", "afternoon", "evening"] as const).map(
      (key) => ({
        key,
        minutes: buckets[key],
        percent: totalMinutes > 0 ? buckets[key] / totalMinutes : 0,
      }),
    );

    // top apps
    const topApps: QuickUsageEntry[] = day.apps.slice(0, 10).map((a) => ({
      key: a.process,
      minutes: a.minutes,
      percent: totalMinutes > 0 ? a.minutes / totalMinutes : 0,
      categoryId: a.categoryId,
      iconProcess: a.iconProcess,
    }));

    // 分类聚合
    const catMap = new Map<string, number>();
    for (const a of day.apps) {
      catMap.set(a.categoryId, (catMap.get(a.categoryId) ?? 0) + a.minutes);
    }
    const categories: QuickUsageEntry[] = Array.from(catMap.entries())
      .map(([categoryId, minutes]) => ({
        key: categoryId,
        minutes,
        percent: totalMinutes > 0 ? minutes / totalMinutes : 0,
        categoryId: "",
        iconProcess: "",
      }))
      .sort((a, b) => b.minutes - a.minutes);

    return {
      date,
      totalMinutes,
      activeHours,
      peakHour: totalMinutes > 0 ? peakHour : null,
      peakHourMinutes,
      dayParts,
      topApps,
      categories,
    };
  },

  getQuickWeekSummary: async (
    weekOffset: number,
    deviceId?: string,
  ): Promise<QuickWeekSummary> => {
    const today = new Date();
    const dow = today.getDay() || 7;
    const monday = new Date(today);
    monday.setDate(today.getDate() - (dow - 1) + weekOffset * 7);
    const sunday = new Date(monday);
    sunday.setDate(monday.getDate() + 6);

    const dailySeries: { date: string; minutes: number }[] = [];
    const appMap = new Map<string, QuickUsageEntry>();
    const catMap = new Map<string, number>();
    let totalMinutes = 0;
    let activeDays = 0;
    let peakDay: QuickPeakDay | null = null;

    for (let i = 0; i < 7; i++) {
      const d = new Date(monday);
      d.setDate(monday.getDate() + i);
      const offset = Math.round((d.getTime() - today.getTime()) / (24 * 3600 * 1000));
      const dayMinutes =
        offset > 0
          ? 0
          : mockDayFor(offset, deviceId).apps.reduce((s, a) => s + a.minutes, 0);
      const isoDate = ymd(d);
      dailySeries.push({ date: isoDate, minutes: dayMinutes });
      if (dayMinutes > 0) {
        activeDays += 1;
        totalMinutes += dayMinutes;
        if (!peakDay || dayMinutes > peakDay.minutes) {
          peakDay = { date: isoDate, minutes: dayMinutes, weekday: i };
        }
        if (offset <= 0) {
          const day = mockDayFor(offset, deviceId);
          for (const a of day.apps) {
            const cur = appMap.get(a.process);
            if (cur) cur.minutes += a.minutes;
            else
              appMap.set(a.process, {
                key: a.process,
                minutes: a.minutes,
                percent: 0,
                categoryId: a.categoryId,
                iconProcess: a.iconProcess,
              });
            catMap.set(a.categoryId, (catMap.get(a.categoryId) ?? 0) + a.minutes);
          }
        }
      }
    }

    const topApps = Array.from(appMap.values())
      .map((e) => ({ ...e, percent: totalMinutes > 0 ? e.minutes / totalMinutes : 0 }))
      .sort((a, b) => b.minutes - a.minutes)
      .slice(0, 10);
    const categories = Array.from(catMap.entries())
      .map(([categoryId, minutes]) => ({
        key: categoryId,
        minutes,
        percent: totalMinutes > 0 ? minutes / totalMinutes : 0,
        categoryId: "",
        iconProcess: "",
      }))
      .sort((a, b) => b.minutes - a.minutes);

    return {
      weekStart: ymd(monday),
      weekEnd: ymd(sunday),
      totalMinutes,
      activeDays,
      dailyAverageMinutes: activeDays > 0 ? Math.round(totalMinutes / activeDays) : 0,
      peakDay,
      dailySeries,
      topApps,
      categories,
    };
  },

  getQuickMonthSummary: async (
    monthOffset: number,
    deviceId?: string,
  ): Promise<QuickMonthSummary> => {
    const today = new Date();
    const target = new Date(today.getFullYear(), today.getMonth() + monthOffset, 1);
    const totalDays = new Date(target.getFullYear(), target.getMonth() + 1, 0).getDate();

    const dailySeries: { date: string; minutes: number }[] = [];
    const appMap = new Map<string, QuickUsageEntry>();
    const catMap = new Map<string, number>();
    let totalMinutes = 0;
    let activeDays = 0;
    let peakDay: QuickPeakDay | null = null;
    let quietDay: QuickPeakDay | null = null;

    for (let i = 0; i < totalDays; i++) {
      const d = new Date(target);
      d.setDate(1 + i);
      const offset = Math.round((d.getTime() - today.getTime()) / (24 * 3600 * 1000));
      const isoDate = ymd(d);
      const wd = (d.getDay() + 6) % 7;
      let dayMinutes = 0;
      if (offset <= 0) {
        const day = mockDayFor(offset, deviceId);
        dayMinutes = day.apps.reduce((s, a) => s + a.minutes, 0);
        if (dayMinutes > 0) {
          for (const a of day.apps) {
            const cur = appMap.get(a.process);
            if (cur) cur.minutes += a.minutes;
            else
              appMap.set(a.process, {
                key: a.process,
                minutes: a.minutes,
                percent: 0,
                categoryId: a.categoryId,
                iconProcess: a.iconProcess,
              });
            catMap.set(a.categoryId, (catMap.get(a.categoryId) ?? 0) + a.minutes);
          }
        }
      }
      dailySeries.push({ date: isoDate, minutes: dayMinutes });
      if (dayMinutes > 0) {
        activeDays += 1;
        totalMinutes += dayMinutes;
        if (!peakDay || dayMinutes > peakDay.minutes) {
          peakDay = { date: isoDate, minutes: dayMinutes, weekday: wd };
        }
        if (!quietDay || dayMinutes < quietDay.minutes) {
          quietDay = { date: isoDate, minutes: dayMinutes, weekday: wd };
        }
      }
    }

    const topApps = Array.from(appMap.values())
      .map((e) => ({ ...e, percent: totalMinutes > 0 ? e.minutes / totalMinutes : 0 }))
      .sort((a, b) => b.minutes - a.minutes)
      .slice(0, 10);
    const categories = Array.from(catMap.entries())
      .map(([categoryId, minutes]) => ({
        key: categoryId,
        minutes,
        percent: totalMinutes > 0 ? minutes / totalMinutes : 0,
        categoryId: "",
        iconProcess: "",
      }))
      .sort((a, b) => b.minutes - a.minutes);

    return {
      monthStart: ymd(target),
      monthEnd: ymd(new Date(target.getFullYear(), target.getMonth(), totalDays)),
      totalDays,
      totalMinutes,
      activeDays,
      dailyAverageMinutes: activeDays > 0 ? Math.round(totalMinutes / activeDays) : 0,
      peakDay,
      // 月内只有 1 个有数据的日子时 quietDay = peakDay 没意义，按合约置 null
      quietDay: activeDays >= 2 ? quietDay : null,
      dailySeries,
      topApps,
      categories,
    };
  },

  // ─── 分类 ──────────────────────────────────
  listCategories: async (): Promise<Category[]> => structuredClone(state.categories),
  createCategory: async (input: CategoryInput): Promise<Category> => {
    const c: Category = {
      id: `c-${Date.now()}`,
      name: input.name,
      color: input.color,
      icon: input.icon,
      builtin: false,
      apps: [],
    };
    state.categories.push(c);
    persist();
    return c;
  },
  updateCategory: async (id: string, patch: CategoryPatch): Promise<void> => {
    const c = state.categories.find((x) => x.id === id);
    if (c) {
      if (patch.name !== undefined) c.name = patch.name;
      if (patch.color !== undefined) c.color = patch.color;
      if (patch.icon !== undefined) c.icon = patch.icon;
      persist();
    }
  },
  deleteCategory: async (id: string): Promise<void> => {
    state.categories = state.categories.filter((x) => x.id !== id);
    persist();
  },
  reorderCategories: async (orderedIds: string[]): Promise<void> => {
    state.categories.sort(
      (a, b) => orderedIds.indexOf(a.id) - orderedIds.indexOf(b.id),
    );
    persist();
  },

  // ─── 应用分配 ──────────────────────────────
  assignApp: async (processName: string, categoryId: string): Promise<void> => {
    // 从其他分类移除
    for (const c of state.categories) c.apps = c.apps.filter((a) => a !== processName);
    const target = state.categories.find((c) => c.id === categoryId);
    if (target && !target.apps.includes(processName)) target.apps.push(processName);
    persist();
  },
  unassignApp: async (processName: string): Promise<void> => {
    for (const c of state.categories) c.apps = c.apps.filter((a) => a !== processName);
    persist();
  },
  listUnclassifiedApps: async (_daysBack?: number): Promise<UnclassifiedApp[]> =>
    structuredClone(mockUnclassifiedApps),

  // ─── App groups ───────────────────────────
  listAppGroups: async (): Promise<AppGroup[]> => structuredClone(state.appGroups),
  createAppGroup: async (displayName: string): Promise<string> => {
    const id = `g-${Date.now()}`;
    state.appGroups.push({ id, displayName, categoryId: null, members: [] });
    persist();
    return id;
  },
  deleteAppGroup: async (groupId: string): Promise<void> => {
    state.appGroups = state.appGroups.filter((g) => g.id !== groupId);
    persist();
  },
  mergeAppGroup: async (processName: string, targetGroupId: string): Promise<void> => {
    const g = state.appGroups.find((x) => x.id === targetGroupId);
    if (g && !g.members.find((m) => m.processName === processName)) {
      g.members.push({ processName, recentSecs: 0, lastDeviceId: "demo-self" });
      persist();
    }
  },
  unmergeAppGroup: async (processName: string): Promise<void> => {
    for (const g of state.appGroups) {
      g.members = g.members.filter((m) => m.processName !== processName);
    }
    persist();
  },
  renameAppGroup: async (groupId: string, displayName: string): Promise<void> => {
    const g = state.appGroups.find((x) => x.id === groupId);
    if (g) {
      g.displayName = displayName;
      persist();
    }
  },
  assignAppGroupCategory: async (
    groupId: string,
    categoryId: string | null,
  ): Promise<void> => {
    const g = state.appGroups.find((x) => x.id === groupId);
    if (g) {
      g.categoryId = categoryId;
      persist();
    }
  },

  // ─── Capture ───────────────────────────────
  startCapture: async (): Promise<void> => {
    state.settings.captureEnabled = true;
    persist();
  },
  stopCapture: async (): Promise<void> => {
    state.settings.captureEnabled = false;
    persist();
  },
  getCaptureStatus: async (): Promise<CaptureStatus> => ({
    running: state.settings.captureEnabled,
    todayCount: 18432,
    lastCaptureAt: new Date().toISOString(),
    lastError: null,
  }),
  getAppIcon: async (processName: string): Promise<string | null> => {
    return APP_ICON_URLS[processName.toLowerCase()] ?? null;
  },

  // ─── Settings ──────────────────────────────
  getSettings: async (): Promise<Settings> => {
    // demo 的 AI 系统提示词语言 + "あなたについて / About you" 简介都按当前 i18n 切，
    // 让 /en/、/ja/ 看到对应语言的默认值。
    const s = structuredClone(state.settings);
    const lng = (i18n.language || "zh-CN").toLowerCase();
    s.ai.promptLanguage = lng.startsWith("ja") ? "ja" : lng.startsWith("zh") ? "zh" : "en";
    s.ai.userBrief = userBriefForLocale(lng);
    // 清空 promptOverrides，确保 PromptTab fallback 到 DEFAULT_SYSTEM_PROMPTS[lang]
    s.ai.promptOverrides = { systemZh: "", systemEn: "", systemJa: "" };
    s.ai.imageDescribeOverrides = { systemZh: "", systemEn: "", systemJa: "" };
    return s;
  },
  updateSettings: async (patch: SettingsPatch): Promise<Settings> => {
    state.settings = { ...state.settings, ...patch };
    if (patch.ai) state.settings.ai = { ...state.settings.ai, ...patch.ai };
    persist();
    return structuredClone(state.settings);
  },
  getStorageInfo: async (): Promise<StorageInfo> => ({
    dbBytes: 42 * 1024 * 1024,
    screenshotsBytes: 1.2 * 1024 * 1024 * 1024,
    dbPath: "C:\\Users\\demo\\AppData\\Roaming\\Hindsight\\hindsight.db",
    screenshotsPath:
      "C:\\Users\\demo\\AppData\\Roaming\\Hindsight\\screenshots",
  }),
  purgeActivities: async (): Promise<void> => {
    // eslint-disable-next-line no-console
    console.warn("[demo] purgeActivities 在 demo 模式下不会真的清除数据");
  },
  purgeScreenshots: async (): Promise<void> => {
    // eslint-disable-next-line no-console
    console.warn("[demo] purgeScreenshots 在 demo 模式下不会真的清除数据");
  },
  openScreenshotsDir: async (): Promise<void> => {},
  getDataRoot: async (): Promise<string> =>
    "C:\\Users\\demo\\AppData\\Roaming\\Hindsight",
  setDataRoot: async (_path: string): Promise<void> => {},

  // ─── Devices ───────────────────────────────
  listDevices: async (): Promise<DeviceRow[]> => structuredClone(state.devices),
  updateSelfDevice: async (
    name?: string,
    color?: string,
    icon?: string,
  ): Promise<DeviceRow> => {
    const self = state.devices.find((d) => d.isSelf);
    if (self) {
      if (name !== undefined) self.displayName = name;
      if (color !== undefined) self.color = color;
      if (icon !== undefined) self.icon = icon;
      persist();
    }
    return structuredClone(self!);
  },
  authStatus: async (): Promise<AuthState> => structuredClone(state.authState),
  signInWithGoogle: async (): Promise<AuthState> => {
    // 模拟登录成功
    state.authState = {
      signedIn: true,
      uid: "demo-uid",
      email: "demo@hindsight.app",
      configured: true,
    };
    return structuredClone(state.authState);
  },
  signOut: async (): Promise<void> => {
    state.authState = structuredClone(mockAuthState);
  },
  restartApp: async (): Promise<void> => {
    if (typeof window !== "undefined") window.location.reload();
  },
  syncStatus: async (): Promise<SyncStatus> => structuredClone(state.syncStatus),
  syncNow: async (): Promise<void> => {
    state.syncStatus.running = true;
    setTimeout(() => {
      state.syncStatus.running = false;
      state.syncStatus.lastPushedAt = new Date().toISOString();
      state.syncStatus.lastPulledAt = new Date().toISOString();
    }, 1500);
  },

  // ─── AI external endpoint ──────────────────
  testAiEndpoint: async (
    _endpoint: string,
    _apiKey?: string,
  ): Promise<TestAiEndpointResp> => ({
    ok: true,
    models: ["gpt-4o-mini", "gpt-4o", "deepseek-chat"],
    message: "Demo 模式 · 假返回",
  }),

  // ─── AI engine ─────────────────────────────
  getEngineStatus: async (): Promise<EngineStatus> => structuredClone(mockEngineStatus),
  downloadBinary: async (): Promise<void> => {
    // 模拟两阶段下载（engine → runtime）
    await simulateEngineDownload();
  },
  deleteBinary: async (): Promise<void> => {},
  openEngineDir: async (): Promise<void> => {},
  getEngineLogs: async (): Promise<string[]> => [
    "[demo] llama-server started on port 8088",
    "[demo] loaded model Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf",
    "[demo] vocab size = 152064",
    "[demo] context window = 8192",
    "[demo] CUDA device: NVIDIA GeForce RTX 4070 Ti",
  ],
  startEngine: async (): Promise<number> => 8088,
  stopEngine: async (): Promise<void> => {},

  // ─── Models ────────────────────────────────
  listLocalModels: async (): Promise<ModelEntry[]> => structuredClone(mockLocalModels),
  deleteModel: async (_filename: string): Promise<void> => {},
  listRecommendedModels: async (): Promise<RecommendedModel[]> =>
    structuredClone(mockRecommendedModels),
  downloadModel: async (
    _repo: string,
    file: string,
    expectedBytes: number,
    saveAs?: string | null,
  ): Promise<string> => {
    await simulateModelDownload(saveAs ?? file, expectedBytes);
    return `C:\\Users\\demo\\AppData\\Roaming\\Hindsight\\ai\\models\\${saveAs ?? file}`;
  },
  cancelModelDownload: async (_file: string): Promise<void> => {},
  listPartialDownloads: async (): Promise<PartialDownload[]> => [],
  setActiveModel: async (
    mainFile: string,
    mmprojFile: string | null,
  ): Promise<void> => {
    state.settings.ai.activeMain = mainFile;
    state.settings.ai.activeMmproj = mmprojFile ?? "";
    persist();
  },
  setStepModel: async (
    step: "describe" | "summary",
    mainFile: string,
    mmprojFile: string | null,
  ): Promise<void> => {
    if (step === "describe") {
      state.settings.ai.describeMain = mainFile;
      state.settings.ai.describeMmproj = mmprojFile ?? "";
    } else {
      state.settings.ai.summaryMain = mainFile;
      state.settings.ai.summaryMmproj = mmprojFile ?? "";
    }
    persist();
  },

  // ─── AI summary ────────────────────────────
  generateDaySummary: async (
    date: string,
    _forceRefresh: boolean,
    _deviceId: string | null,
    _overrides: AiOverrides | null = null,
    source: string = "daily",
    _step1Only: boolean = false,
    _step2Only: boolean = false,
  ): Promise<void> => {
    await simulateSummaryProgress(date, source);
  },
  retrySummarySegment: async (
    date: string,
    segmentIdx: number,
    _deviceId: string | null,
    _overrides: AiOverrides | null = null,
    source: string = "daily",
  ): Promise<void> => {
    await simulateSegmentRetry(date, segmentIdx, source);
  },
  cancelDaySummary: async (): Promise<void> => {},
  getDaySummary: async (
    date: string,
    source: string = "daily",
  ): Promise<SegmentSummaryRow[]> => {
    const key = `${source}:${date}`;
    // 只有 daily 段总结按当前 i18n 语言动态返回；其它 source（weekly/monthly）保持
    // 落在 state 里的拷贝。这样切语言后日报立刻跟随，不需要清缓存。
    if (state.daySummaries.has(key) && source === "daily") {
      const segs = dailySegmentsForLocale(i18n.language || "zh-CN");
      for (const s of segs) s.localDate = date;
      return segs;
    }
    return structuredClone(state.daySummaries.get(key) ?? []);
  },

  // ─── Weekly summary ────────────────────────
  generateWeekSummary: async (
    _weekStart: string,
    _forceRefresh: boolean,
    _allowMissingDays: boolean = false,
  ): Promise<void> => {
    // 简化：1.5s 后 emit done
    await new Promise((r) => setTimeout(r, 1500));
  },
  precheckWeekSummary: async (weekStart: string): Promise<WeekPrecheckResp> => {
    return {
      days: Array.from({ length: 7 }, (_, i) => {
        const d = new Date(weekStart);
        d.setDate(d.getDate() + i);
        return {
          date: `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`,
          weekday: ["周一", "周二", "周三", "周四", "周五", "周六", "周日"][i],
          hasDaily: i < 5, // 前 5 天有日报
          hasActivity: true,
        };
      }),
      daysWithDaily: 5,
      daysActivityOnly: 2,
    };
  },
  getWeekSummary: async (_weekStart: string): Promise<SegmentSummaryRow | null> => null,
  clearWeekSummary: async (_weekStart: string): Promise<void> => {},

  // ─── AI image descriptions ─────────────────
  getSegmentImageDescriptions: async (
    _date: string,
    _segmentIdx: number,
    _source: string = "daily",
  ): Promise<ImageDescriptionRow[]> => [],
  getDayImageDescriptions: async (
    _date: string,
    _source: string = "daily",
  ): Promise<ImageDescriptionRow[]> => [],
  clearDaySummary: async (date: string, source: string = "daily"): Promise<void> => {
    state.daySummaries.delete(`${source}:${date}`);
  },
  clearDayImageDescriptions: async (
    _date: string,
    _source: string = "daily",
  ): Promise<void> => {},
  clearDaySegmentSummaries: async (
    date: string,
    source: string = "daily",
  ): Promise<void> => {
    state.daySummaries.delete(`${source}:${date}`);
  },
  retrySingleImageDescription: async (
    _date: string,
    _segmentIdx: number,
    _imageIndex: number,
    _overrides: AiOverrides | null = null,
    _source: string = "daily",
  ): Promise<void> => {},
};

// ────────────────────────────────────────────
// 长任务模拟器（事件驱动）
// ────────────────────────────────────────────

const SUMMARY_EVENT = "ai://summary-progress";
const MODEL_EVENT = "ai://model-download-progress";
const ENGINE_EVENT = "ai://engine-download-progress";

async function simulateSummaryProgress(date: string, source: string): Promise<void> {
  // 5 段，每段 ~800ms：emit started → emit done。按当前 i18n locale 取段内容。
  const segments = dailySegmentsForLocale(i18n.language || "zh-CN");
  for (const seg of segments) seg.localDate = date;

  await emit(SUMMARY_EVENT, {
    phase: "engine_starting",
    message: "加载段总结模型中…",
    source,
  });
  await sleep(400);

  for (let i = 0; i < segments.length; i++) {
    await emit(SUMMARY_EVENT, {
      phase: "segment_started",
      date,
      segmentIdx: i,
      source,
    });
    await emit(SUMMARY_EVENT, {
      phase: "summarizing",
      date,
      segmentIdx: i,
      source,
    });
    await sleep(700);
    state.daySummaries.set(`${source}:${date}`, segments.slice(0, i + 1));
    await emit(SUMMARY_EVENT, {
      phase: "segment_done",
      date,
      segmentIdx: i,
      status: "ok",
      content: segments[i].content,
      source,
    });
  }

  await emit(SUMMARY_EVENT, { phase: "all_done", date, source });
}

async function simulateSegmentRetry(
  date: string,
  segmentIdx: number,
  source: string,
): Promise<void> {
  await emit(SUMMARY_EVENT, {
    phase: "segment_started",
    date,
    segmentIdx,
    source,
  });
  await sleep(800);
  const segments = state.daySummaries.get(`${source}:${date}`) ?? [];
  if (segments[segmentIdx]) {
    await emit(SUMMARY_EVENT, {
      phase: "segment_done",
      date,
      segmentIdx,
      status: "ok",
      content: segments[segmentIdx].content,
      source,
    });
  }
}

async function simulateModelDownload(file: string, totalBytes: number): Promise<void> {
  const steps = 20;
  const step = totalBytes / steps;
  for (let i = 0; i <= steps; i++) {
    await emit(MODEL_EVENT, {
      file,
      downloaded: Math.min(totalBytes, Math.round(step * i)),
      total: totalBytes,
    });
    await sleep(120);
  }
}

async function simulateEngineDownload(): Promise<void> {
  // engine 阶段
  const engineTotal = 220 * 1024 * 1024;
  for (let i = 0; i <= 15; i++) {
    await emit(ENGINE_EVENT, {
      phase: "downloading",
      downloaded: Math.round((engineTotal / 15) * i),
      total: engineTotal,
      stage: "engine",
    });
    await sleep(80);
  }
  await emit(ENGINE_EVENT, {
    phase: "extracting",
    downloaded: engineTotal,
    total: engineTotal,
    stage: "engine",
  });
  await sleep(300);

  // runtime 阶段
  const runtimeTotal = 28 * 1024 * 1024;
  for (let i = 0; i <= 10; i++) {
    await emit(ENGINE_EVENT, {
      phase: "downloading",
      downloaded: Math.round((runtimeTotal / 10) * i),
      total: runtimeTotal,
      stage: "runtime",
    });
    await sleep(60);
  }
  await emit(ENGINE_EVENT, {
    phase: "done",
    downloaded: runtimeTotal,
    total: runtimeTotal,
    stage: "runtime",
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
