// Demo 路由懒加载定义与预加载调度（纯 web/demo-src 内部，不改主仓库 src/）。
// 配合 AppDemo 的 hover/focus 事件委托和空闲后台预热，消除网页端首次点击侧栏 / Tab 的加载等待。
import { lazy, type ComponentType, type LazyExoticComponent } from "react";
import { ROUTES } from "@app/config/nav";

type PageModule = { default: ComponentType };

export interface PreloadableRoute {
  Component: LazyExoticComponent<ComponentType>;
  preload: () => Promise<PageModule>;
}

function createPreloadable(loader: () => Promise<PageModule>): PreloadableRoute {
  let cached: Promise<PageModule> | null = null;
  const preload = () => {
    if (!cached) {
      cached = loader().catch((err) => {
        // 加载失败时清掉缓存，允许用户再次点击重试
        cached = null;
        throw err;
      });
    }
    return cached;
  };
  return {
    Component: lazy(preload),
    preload,
  };
}

export const routeChunks = {
  week: createPreloadable(() => import("@app/pages/Week/WeekPage")),
  month: createPreloadable(() => import("@app/pages/Month/MonthPage")),
  history: createPreloadable(() => import("@app/pages/History/HistoryPage")),
  historyStats: createPreloadable(() => import("@app/pages/History/StatsTab")),
  search: createPreloadable(() => import("@app/pages/Search/SearchPage")),
  chat: createPreloadable(() => import("@app/pages/Chat/ChatPage")),
  aiSummary: createPreloadable(() => import("@app/pages/AISummary/AISummaryPage")),
  aiDaily: createPreloadable(() => import("@app/pages/AISummary/tabs/DailyTab")),
  aiWeekly: createPreloadable(() => import("@app/pages/AISummary/tabs/WeeklyTab")),
  aiMonthly: createPreloadable(() => import("@app/pages/AISummary/tabs/MonthlyTab")),
  aiDebug: createPreloadable(() => import("@app/pages/AISummary/tabs/DebugTab")),
  aiSettings: createPreloadable(() => import("@app/pages/AISettings/AISettingsPage")),
  aiModels: createPreloadable(() => import("@app/pages/AISettings/tabs/ModelsTab")),
  aiGeneral: createPreloadable(() => import("@app/pages/AISettings/tabs/GeneralTab")),
  aiPrompt: createPreloadable(() => import("@app/pages/AISettings/tabs/PromptTab")),
  aiExternal: createPreloadable(() => import("@app/pages/AISettings/tabs/ExternalApiTab")),
  aiEngine: createPreloadable(() => import("@app/pages/AISettings/tabs/EngineTab")),
  devices: createPreloadable(() => import("@app/pages/Devices/DevicesPage")),
  categories: createPreloadable(() => import("@app/pages/Categories/CategoriesPage")),
  categoriesList: createPreloadable(() => import("@app/pages/Categories/tabs/ListTab")),
  apps: createPreloadable(() => import("@app/pages/Apps/AppsPage")),
  sites: createPreloadable(() => import("@app/pages/Sites/SitesPage")),
  settings: createPreloadable(() => import("@app/pages/Settings/SettingsPage")),
  settingsGeneral: createPreloadable(() => import("@app/pages/Settings/tabs/GeneralTab")),
  settingsAppearance: createPreloadable(() => import("@app/pages/Settings/tabs/AppearanceTab")),
  settingsData: createPreloadable(() => import("@app/pages/Settings/tabs/DataTab")),
  settingsPrivacy: createPreloadable(() => import("@app/pages/Settings/tabs/PrivacyTab")),
  settingsPlugins: createPreloadable(() => import("@app/pages/Settings/tabs/PluginsTab")),
  settingsAbout: createPreloadable(() => import("@app/pages/Settings/tabs/AboutTab")),
} as const;

export type RouteChunkKey = keyof typeof routeChunks;

/** 去掉 query/hash 及尾部斜杠，统一成路由匹配用 pathname */
export function normalizeRoutePath(to: string): string {
  const clean = to.split(/[?#]/)[0] || "/";
  if (clean.length > 1 && clean.endsWith("/")) return clean.slice(0, -1);
  return clean;
}

/** 路径 → 需要同时预加载的 chunk key（包含父布局页 + 对应子 Tab） */
export function chunkKeysForPath(to: string): RouteChunkKey[] {
  const path = normalizeRoutePath(to);

  switch (path) {
    case ROUTES.week:
      return ["week"];
    case ROUTES.month:
      return ["month"];
    case ROUTES.history:
      return ["history", "historyStats"];
    case `${ROUTES.history}/search`:
      return ["history", "search"];
    case ROUTES.chat:
      return ["chat"];
    case ROUTES.aiSummary:
      return ["aiSummary", "aiDaily"];
    case `${ROUTES.aiSummary}/week`:
      return ["aiSummary", "aiWeekly"];
    case `${ROUTES.aiSummary}/month`:
      return ["aiSummary", "aiMonthly"];
    case `${ROUTES.aiSummary}/debug`:
      return ["aiSummary", "aiDebug"];
    case ROUTES.aiSettings:
    case `${ROUTES.aiSettings}/models`:
      return ["aiSettings", "aiModels"];
    case `${ROUTES.aiSettings}/general`:
      return ["aiSettings", "aiGeneral"];
    case `${ROUTES.aiSettings}/prompt`:
      return ["aiSettings", "aiPrompt"];
    case `${ROUTES.aiSettings}/external`:
      return ["aiSettings", "aiExternal"];
    case ROUTES.devices:
      return ["devices"];
    case ROUTES.categories:
      return ["categories", "categoriesList"];
    case `${ROUTES.categories}/apps`:
    case ROUTES.apps:
      return ["categories", "apps"];
    case `${ROUTES.categories}/sites`:
      return ["categories", "sites"];
    case ROUTES.settings:
      return ["settings", "settingsGeneral"];
    case `${ROUTES.settings}/appearance`:
      return ["settings", "settingsAppearance"];
    case `${ROUTES.settings}/data`:
      return ["settings", "settingsData"];
    case `${ROUTES.settings}/privacy`:
      return ["settings", "settingsPrivacy"];
    case `${ROUTES.settings}/engine`:
      return ["settings", "aiEngine"];
    case `${ROUTES.settings}/plugins`:
      return ["settings", "settingsPlugins"];
    case `${ROUTES.settings}/about`:
      return ["settings", "settingsAbout"];
    default:
      return [];
  }
}

/** 预加载指定路由路径对应的页面及子 Tab chunk */
export function preloadRoute(to: string): void {
  for (const key of chunkKeysForPath(to)) {
    void routeChunks[key].preload().catch(() => {});
  }
}

/** 侧边栏所有一级入口，供首屏完成后空闲预热 */
const IDLE_PRELOAD_PATHS: readonly string[] = [
  ROUTES.week,
  ROUTES.month,
  ROUTES.history,
  ROUTES.aiSummary,
  ROUTES.chat,
  ROUTES.categories,
  ROUTES.devices,
  ROUTES.settings,
];

/**
 * 在浏览器空闲时段逐批后台预加载侧边栏核心页面，消除网页端（含无 hover 的移动端）
 * 首次点击菜单时等待懒加载 chunk 的明显卡顿。返回取消函数。
 */
export function scheduleIdleRoutePreload(): () => void {
  if (typeof window === "undefined") return () => {};

  let cancelled = false;
  let timerId: number | null = null;
  let idleId: number | null = null;
  let index = 0;

  const win = window as Window & {
    requestIdleCallback?: (cb: () => void, opts?: { timeout: number }) => number;
    cancelIdleCallback?: (id: number) => void;
  };

  const runNext = () => {
    if (cancelled || index >= IDLE_PRELOAD_PATHS.length) return;
    // 每次空闲只预热 1 个一级入口，避免并发请求过多挤占其它高优先级请求（如主题预览 JSON）
    const nextPath = IDLE_PRELOAD_PATHS[index++];
    const tasks = chunkKeysForPath(nextPath).map((k) => routeChunks[k].preload().catch(() => {}));
    void Promise.all(tasks).then(() => {
      if (!cancelled && index < IDLE_PRELOAD_PATHS.length) {
        scheduleNext();
      }
    });
  };

  const scheduleNext = () => {
    if (typeof win.requestIdleCallback === "function") {
      idleId = win.requestIdleCallback(runNext, { timeout: 1500 });
    } else {
      timerId = window.setTimeout(runNext, 200);
    }
  };

  // 等首屏渲染与初始网络请求完全落定后再开始后台预热，不与首屏抢带宽
  timerId = window.setTimeout(scheduleNext, 800);

  return () => {
    cancelled = true;
    if (timerId !== null) window.clearTimeout(timerId);
    if (idleId !== null && typeof win.cancelIdleCallback === "function") {
      win.cancelIdleCallback(idleId);
    }
  };
}
