// API mock —— 替换 src/api/hindsight.ts 给 demo 用。
//
// 设计要点：
// - 类型 100% 复用主仓库（type-only import），保证 demo 编译跟主应用同步
// - 只读方法：从 fixtures 返数据（一些写操作改内存 + localStorage）
// - 长任务（AI 总结、模型下载）：用 setTimeout 模拟阶段性 emit 进度事件
// - 错误处理：尽量"成功"——demo 不应该让用户看到错误状态破坏体验

import type {
  AppDetail,
  AppGroup,
  AppUsage,
  AuthState,
  Category,
  CategoryInput,
  CategoryPatch,
  CaptureStatus,
  ChatAskResult,
  ChatConversationMeta,
  ChatStoredMessage,
  DaySummary,
  DaySummaryDto,
  DeviceRow,
  DigestReport,
  EngineStatus,
  HourSlot,
  MemoryPendingStats,
  MemorySearchResp,
  ModelEntry,
  PartialDownload,
  RecommendedModel,
  SegmentSummaryRow,
  Settings,
  SettingsPatch,
  StorageInfo,
  SuperCategory,
  SuperCategoryInput,
  SuperCategoryPatch,
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
/** demo 跟主 API hindsight.ts 同步：summary_main = "__cloud__" 表示 step 2 走云端。
 *  demo 里没真云端调用，仅作为类型 / 比较符号存在，让消费方代码能跑通。 */
export const SUMMARY_CLOUD_SENTINEL = "__cloud__";
/** OAuth 授权 URL 就绪事件——demo 里永不触发，仅满足消费方 import。 */
export const OAUTH_URL_EVENT = "sync://oauth-url";
/** Chat 问答落库完成广播——demo 里永不触发，仅满足消费方 import。 */
export const CHAT_ANSWER_READY_EVENT = "chat:answer-ready";

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
  mockSuperCategories,
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
  superCategories: SuperCategory[];
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
    superCategories: structuredClone(mockSuperCategories),
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
          secs: minutes * 60,
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
          secs: minutes * 60,
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
      superCategoryId: null,
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

  // ─── 大类（super-category）—— v28+ ────────
  // 主仓库的 useSuperCategoriesProvider 启动时无条件调 listSuperCategories；
  // demo 必须实现这一组，否则 useSuperCategories 抛错 → 整树 unmount
  listSuperCategories: async (): Promise<SuperCategory[]> =>
    structuredClone(state.superCategories),
  createSuperCategory: async (input: SuperCategoryInput): Promise<SuperCategory> => {
    const s: SuperCategory = {
      id: `sup-${Date.now()}`,
      name: input.name,
      color: input.color,
      icon: input.icon,
      sortOrder: state.superCategories.length,
    };
    state.superCategories.push(s);
    return s;
  },
  updateSuperCategory: async (
    id: string,
    patch: SuperCategoryPatch,
  ): Promise<void> => {
    const s = state.superCategories.find((x) => x.id === id);
    if (s) {
      if (patch.name !== undefined) s.name = patch.name;
      if (patch.color !== undefined) s.color = patch.color;
      if (patch.icon !== undefined) s.icon = patch.icon;
    }
  },
  reorderSuperCategories: async (orderedIds: string[]): Promise<void> => {
    state.superCategories.sort(
      (a, b) => orderedIds.indexOf(a.id) - orderedIds.indexOf(b.id),
    );
  },
  deleteSuperCategory: async (id: string): Promise<void> => {
    state.superCategories = state.superCategories.filter((x) => x.id !== id);
    // 子分类 super_category_id 置 null（跟 Rust 端语义一致）
    for (const c of state.categories) {
      if (c.superCategoryId === id) c.superCategoryId = null;
    }
    persist();
  },
  assignCategoryToSuper: async (
    categoryId: string,
    superId: string | null,
  ): Promise<void> => {
    const c = state.categories.find((x) => x.id === categoryId);
    if (c) {
      c.superCategoryId = superId;
      persist();
    }
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
    s.ai.promptOverrides = {
      systemZh: "",
      systemEn: "",
      systemJa: "",
      systemPt: "",
      systemTw: "",
    };
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
  downloadBinary: async (_force = false): Promise<void> => {
    await simulateEngineDownload();
  },
  deleteBinary: async (): Promise<void> => {},
  downloadOcrRuntime: async (_force = false): Promise<void> => {
    // demo 里 runtime 视作已装,直接返回
  },
  deleteOcrRuntime: async (): Promise<void> => {},
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
    step: "summary" | "chat",
    mainFile: string,
    mmprojFile: string | null,
  ): Promise<void> => {
    if (step === "chat") {
      state.settings.ai.chatMain = mainFile;
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

  clearDaySummary: async (date: string, source: string = "daily"): Promise<void> => {
    state.daySummaries.delete(`${source}:${date}`);
  },
  clearDaySegmentSummaries: async (
    date: string,
    source: string = "daily",
  ): Promise<void> => {
    state.daySummaries.delete(`${source}:${date}`);
  },
  // ─── 应用详情钻取（日 24 小时柱 / 周 7 天柱 / 月 30 天柱） ───
  getAppDayDetail: async (
    _dayOffset: number,
    iconProcess: string,
    _deviceId?: string,
  ): Promise<AppDetail> => mockAppDetail("hours", 24, iconProcess),
  getAppWeekDetail: async (
    _weekOffset: number,
    iconProcess: string,
    _deviceId?: string,
  ): Promise<AppDetail> => mockAppDetail("days", 7, iconProcess),
  getAppMonthDetail: async (
    _monthOffset: number,
    iconProcess: string,
    _deviceId?: string,
  ): Promise<AppDetail> => mockAppDetail("days", 30, iconProcess),

  // ─── Chat（演示回答） ────────────────────
  chatAsk: async (
    question: string,
    conversationId: number | null,
    _locale?: string,
    _askId?: string,
  ): Promise<ChatAskResult> => {
    await sleep(900); // 模拟检索 + 推理耗时
    const now = new Date().toISOString();
    let conv = chatConvs.find((c) => c.meta.id === conversationId);
    if (!conv) {
      conv = {
        meta: {
          id: chatNextConvId++,
          title: question.length > 24 ? `${question.slice(0, 24)}…` : question,
          createdTs: now,
          updatedTs: now,
        },
        messages: [],
      };
      chatConvs.unshift(conv);
    }
    conv.meta.updatedTs = now;
    const answer = chatDemoAnswer(question);
    conv.messages.push({
      id: chatNextMsgId++,
      role: "user",
      content: question,
      citations: [],
      degraded: false,
      createdTs: now,
      promptTokens: null,
      completionTokens: null,
    });
    conv.messages.push({
      id: chatNextMsgId++,
      role: "assistant",
      content: answer.text,
      citations: answer.citations,
      degraded: false,
      createdTs: now,
      promptTokens: 1284,
      completionTokens: 236,
    });
    return {
      conversationId: conv.meta.id,
      cancelled: false,
      text: answer.text,
      citations: answer.citations,
      steps: 2,
      degraded: false,
      promptTokens: 1284,
      completionTokens: 236,
    };
  },
  chatInflight: async (_conversationId: number): Promise<string | null> => null,
  chatCancel: async (_askId: string): Promise<boolean> => false,
  chatListConversations: async (): Promise<ChatConversationMeta[]> =>
    chatConvs.map((c) => structuredClone(c.meta)),
  chatGetMessages: async (conversationId: number): Promise<ChatStoredMessage[]> =>
    structuredClone(chatConvs.find((c) => c.meta.id === conversationId)?.messages ?? []),
  chatRenameConversation: async (conversationId: number, title: string): Promise<void> => {
    const conv = chatConvs.find((c) => c.meta.id === conversationId);
    if (conv) {
      conv.meta.title = title;
      conv.meta.updatedTs = new Date().toISOString();
    }
  },
  chatDeleteConversation: async (conversationId: number): Promise<void> => {
    const i = chatConvs.findIndex((c) => c.meta.id === conversationId);
    if (i >= 0) chatConvs.splice(i, 1);
  },

  // ─── 屏幕记忆（demo 无积压） ──────────────
  memoryPendingStats: async (): Promise<MemoryPendingStats> => ({
    unregistered: 0,
    pendingOcr: 0,
    total: 0,
    digestRunning: false,
  }),
  memoryBackfill: async (): Promise<number> => 0,
  memoryDigestNow: async (): Promise<DigestReport> => ({
    processed: 0,
    failed: 0,
    skippedMissingFile: 0,
  }),
  memoryDigestStop: async (): Promise<void> => {},
  memorySearch: async (
    query: string,
    _limit?: number,
    offset?: number,
  ): Promise<MemorySearchResp> => {
    await sleep(300); // 模拟索引查询耗时
    if ((offset ?? 0) > 0) return { total: 3, hits: [] };
    return { total: 3, hits: demoSearchHits(query) };
  },
  memoryLocate: async (
    _path: string,
    _words: string[],
  ): Promise<[number, number, number, number][]> => [],
  memorySessionText: async (_sessionId: number): Promise<string> =>
    DEMO_SESSION_TEXT,

  // ─── 杂项 no-op ──────────────────────────
  writeTextFile: async (_path: string, _content: string): Promise<void> => {},
  exportUsageXlsx: async (_path: string, _spec: unknown): Promise<void> => {},
  earliestActivityDate: async (): Promise<string | null> => isoDateOffset(-29),
  setTrayLabels: async (_show: string, _quit: string): Promise<void> => {},
  testAiChat: async (
    _endpoint: string,
    _apiKey: string | undefined,
    _model: string,
    _withImage: boolean,
  ): Promise<TestAiEndpointResp> => ({
    ok: true,
    models: [],
    message: "Demo 模式 · 假返回",
  }),
  importModel: async (srcPath: string): Promise<string> => srcPath,
  purgeAppGroup: async (_groupId: string): Promise<void> => {},
  purgeCloudData: async (_keepLocal: boolean): Promise<void> => {},
  forgetRemoteDevice: async (deviceId: string): Promise<void> => {
    const i = state.devices.findIndex((d) => d.deviceId === deviceId && !d.isSelf);
    if (i >= 0) state.devices.splice(i, 1);
  },
};

// ────────────────────────────────────────────
// Chat 演示状态 + 固定示例回答（按界面语言）
// ────────────────────────────────────────────

interface DemoConv {
  meta: ChatConversationMeta;
  messages: ChatStoredMessage[];
}
const chatConvs: DemoConv[] = [];
let chatNextConvId = 1;
let chatNextMsgId = 1;

function chatDemoAnswer(question: string): {
  text: string;
  citations: ChatStoredMessage["citations"];
} {
  const lng = (i18n.language || "zh-CN").toLowerCase();
  const q = question.length > 40 ? `${question.slice(0, 40)}…` : question;
  const text = lng.startsWith("zh-tw") || lng.startsWith("zh-hk")
    ? `這是展示環境的範例回答。正式版會檢索你的**活動記錄**與**螢幕文字索引**來回答「${q}」:先用統計工具彙總相關應用程式的使用時長與次數,再用全文搜尋找出螢幕上出現過的相關內容,並附上可核對的證據卡 [1,2]。`
    : lng.startsWith("zh")
    ? `这是演示环境的示例回答。正式版会检索你的**活动记录**与**屏幕文字索引**来回答「${q}」:先用统计工具汇总相关应用的使用时长与次数,再用全文搜索找出屏幕上出现过的相关内容,并附上可核对的证据卡 [1,2]。`
    : lng.startsWith("ja")
      ? `これはデモ環境のサンプル回答です。製品版では**アクティビティ記録**と**画面テキスト索引**を検索して「${q}」に回答します:統計ツールで使用時間や回数を集計し、全文検索で画面に表示された内容を見つけ、検証可能な出典カード [1,2] を添付します。`
      : lng.startsWith("pt")
        ? `Esta é uma resposta de demonstração. Na versão real, eu pesquisaria seu **registro de atividades** e o **índice de texto da tela** para responder "${q}": agregando tempo de uso com ferramentas de estatística e buscando conteúdo que apareceu na tela, com cartões de evidência verificáveis [1,2].`
        : `This is a sample answer in the demo environment. The real app would search your **activity records** and **screen-text index** to answer "${q}": aggregating app usage with the stats tool, then full-text searching what appeared on screen, with verifiable evidence cards [1,2].`;
  const today = todayStr();
  return {
    text,
    citations: [
      {
        index: 1,
        app: "Visual Studio Code",
        title: "hindsight — src/pages/Chat/ChatPage.tsx",
        startedTs: `${today}T10:12:00+08:00`,
        endedTs: `${today}T11:03:00+08:00`,
        framePath: null,
      },
      {
        index: 2,
        app: "Google Chrome",
        title: "llama.cpp server docs — GitHub",
        startedTs: `${today}T14:20:00+08:00`,
        endedTs: `${today}T14:41:00+08:00`,
        framePath: null,
      },
    ],
  };
}

/** 搜索页演示命中:snippet 嵌入查询词保证高亮生效;framePath 为 null 走文字降级视图。 */
function demoSearchHits(query: string): MemorySearchResp["hits"] {
  const lng = (i18n.language || "zh-CN").toLowerCase();
  const zh = lng.startsWith("zh");
  const today = todayStr();
  const yesterday = isoDateOffset(-1);
  const snip = (before: string, after: string) => `${before}${query}${after}`;
  return [
    {
      sessionId: 1,
      app: "Visual Studio Code",
      title: "hindsight — src/pages/Chat/ChatPage.tsx",
      startedTs: `${today}T10:12:00+08:00`,
      endedTs: `${today}T11:03:00+08:00`,
      snippet: zh
        ? snip("…const answer = await api.chatAsk(question) // 处理 ", " 的检索逻辑,附证据卡…")
        : snip("…const answer = await api.chatAsk(question) // retrieval logic for ", " with evidence cards…"),
      framePath: null,
      frameTs: null,
    },
    {
      sessionId: 2,
      app: "Google Chrome",
      title: zh ? `${query} — 搜索结果` : `${query} — Search results`,
      startedTs: `${today}T14:20:00+08:00`,
      endedTs: `${today}T14:41:00+08:00`,
      snippet: zh
        ? snip("…关于 ", " 的文档与讨论:实现方式、常见问题与最佳实践…")
        : snip("…docs and discussions about ", ": implementation notes, FAQs and best practices…"),
      framePath: null,
      frameTs: null,
    },
    {
      sessionId: 3,
      app: "Obsidian",
      title: zh ? "工作笔记 — 2026-07" : "Work notes — 2026-07",
      startedTs: `${yesterday}T16:05:00+08:00`,
      endedTs: `${yesterday}T16:22:00+08:00`,
      snippet: zh
        ? snip("…TODO: 整理 ", " 相关的资料,周五前给出结论…")
        : snip("…TODO: collect notes on ", " and summarize by Friday…"),
      framePath: null,
      frameTs: null,
    },
  ];
}

/** 会话 OCR 全文的演示文本(截图降级视图):模拟一屏编辑器内容。 */
const DEMO_SESSION_TEXT = [
  "hindsight — src/pages/Chat/ChatPage.tsx — Visual Studio Code",
  "EXPLORER    src > pages > Chat > ChatPage.tsx",
  "import { useState } from \"react\";",
  "import { api } from \"../../api/hindsight\";",
  "",
  "export function ChatPage() {",
  "  const [question, setQuestion] = useState(\"\");",
  "  const answer = await api.chatAsk(question, activeId);",
  "  // 渲染回答气泡与证据卡",
  "}",
  "",
  "PROBLEMS  OUTPUT  TERMINAL      Ln 42, Col 7  UTF-8  TypeScript",
].join("\n");

/** 应用详情的演示数据:确定性钟形分布(刷新不跳),标题列表用领域合理的假标题。 */
function mockAppDetail(
  kind: "hours" | "days",
  count: number,
  iconProcess: string,
): AppDetail {
  const buckets = Array.from({ length: count }, (_, i) => {
    const key = kind === "hours" ? String(i) : isoDateOffset(i - count + 1);
    // 小时粒度:工作时段高、深夜为 0;天粒度:伪随机但确定
    const w =
      kind === "hours"
        ? i >= 9 && i <= 18
          ? 0.6 + ((i * 3) % 5) / 10
          : i >= 20 && i <= 23
            ? 0.3
            : 0
        : 0.3 + (((i * 7) % 10) / 10) * 0.7;
    return { key, secs: Math.round(w * 3000) };
  });
  const name = iconProcess.replace(/\.exe$/i, "");
  return {
    buckets,
    titles: [
      { title: `hindsight — ${name}`, secs: 5400 },
      { title: `docs / design notes — ${name}`, secs: 2700 },
      { title: "GitHub — Pull Request #42", secs: 1500 },
    ],
  };
}

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
