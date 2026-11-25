// Demo 根组件 —— 接入真 Sidebar + Router + 主应用页面。
// 跟主 main.tsx 的差异：
// 1. 用 MemoryRouter 而不是 BrowserRouter（iframe 内 URL bar 无意义，避免 base path 问题）
// 2. 不挂 WindowControls（iframe 内无窗口控件需要）
// 3. data-platform 强制 "windows"；`?platform=macos` 时按 macOS 窗口画（见 MACOS_WINDOW）

import { Suspense, useEffect, useRef, useState, type SyntheticEvent } from "react";
import { useTranslation } from "react-i18next";
import { MemoryRouter, Routes, Route, Navigate, useNavigate } from "react-router-dom";

import { SettingsProvider } from "@app/state/settings";
import { UpdaterProvider } from "@app/state/updater";
import { CategoriesProvider } from "@app/state/categories";
import { SuperCategoriesProvider } from "@app/state/superCategories";
import { DeviceFilterProvider } from "@app/state/deviceFilter";
import { Sidebar } from "@app/components/Sidebar/Sidebar";
import { ROUTES } from "@app/config/nav";
import TodayPage from "@app/pages/Today/TodayPage";

import "@app/i18n";
import "@app/styles/global.css";
import "@app/styles/tokens.css";

import styles from "./AppDemo.module.css";
import { DEMO_RESET_ROUTE_EVENT } from "./preview-theme";
import { preloadRoute, routeChunks, scheduleIdleRoutePreload } from "./routeChunks";
import { TrafficLights } from "./TrafficLights";

/** `?platform=macos`：像 macOS 应用一样给左上角的红绿灯留出位置，并由 demo 画出来。
 *  文档截图用（scripts/shots/stats.mjs）。 */
const MACOS_WINDOW =
  typeof location !== "undefined" &&
  new URLSearchParams(location.search).get("platform") === "macos";

// Demo 非首屏路由懒加载表（TodayPage 为首屏必经页面，直接静态导入避免首屏多一轮瀑布流）；
// 配合下方事件委托 hover/focus 预加载与空闲预热消除首次点击延迟
const WeekPage = routeChunks.week.Component;
const MonthPage = routeChunks.month.Component;
const AISummaryPage = routeChunks.aiSummary.Component;
const DailyTab = routeChunks.aiDaily.Component;
const WeeklyTab = routeChunks.aiWeekly.Component;
const MonthlyTab = routeChunks.aiMonthly.Component;
const ChatPage = routeChunks.chat.Component;
const HistoryPage = routeChunks.history.Component;
const HistoryStatsTab = routeChunks.historyStats.Component;
const SearchPage = routeChunks.search.Component;
const AISettingsPage = routeChunks.aiSettings.Component;
const ModelsTab = routeChunks.aiModels.Component;
const AiGeneralTab = routeChunks.aiGeneral.Component;
const PromptTab = routeChunks.aiPrompt.Component;
const DevicesPage = routeChunks.devices.Component;
const CategoriesPage = routeChunks.categories.Component;
const ListTab = routeChunks.categoriesList.Component;
const AppsPage = routeChunks.apps.Component;
const SitesPage = routeChunks.sites.Component;
const SettingsPage = routeChunks.settings.Component;
const GeneralTab = routeChunks.settingsGeneral.Component;
const AppearanceTab = routeChunks.settingsAppearance.Component;
const DataTab = routeChunks.settingsData.Component;
const PrivacyTab = routeChunks.settingsPrivacy.Component;
const PluginsTab = routeChunks.settingsPlugins.Component;
const AiModelsTab = routeChunks.settingsAiModels.Component;
const AboutTab = routeChunks.settingsAbout.Component;

/** 通过容器事件委托捕获任意站内 <Link>/<NavLink> 的 hover/focus，无需改动主仓库组件即可预加载 */
function preloadLinkTarget(e: SyntheticEvent<HTMLElement>): void {
  const el = e.target instanceof Element ? e.target.closest("a[href]") : null;
  const href = el?.getAttribute("href");
  if (href && href.startsWith("/")) {
    preloadRoute(href);
  }
}

function DemoLayout() {
  const { i18n } = useTranslation();
  const navigate = useNavigate();
  const contentRef = useRef<HTMLElement | null>(null);
  // 和主应用的 App.tsx 一样：<html lang> 跟着界面语言，CSS 按它调侧栏的宽度
  useEffect(() => {
    document.documentElement.lang = i18n.language;
  }, [i18n.language]);

  // 首屏挂载后在浏览器空闲时段静默预热侧栏主页面 chunk，消除首次点击的公网加载延迟
  useEffect(() => scheduleIdleRoutePreload(), []);

  // 主题页复用常驻 iframe 切换新主题预览时，回到默认首页并滚回顶部，保持与新建 iframe 一致
  useEffect(() => {
    const onReset = () => {
      void navigate(ROUTES.today, { replace: true });
      if (contentRef.current) contentRef.current.scrollTop = 0;
    };
    window.addEventListener(DEMO_RESET_ROUTE_EVENT, onReset);
    return () => window.removeEventListener(DEMO_RESET_ROUTE_EVENT, onReset);
  }, [navigate]);

  return (
    <div
      className={styles.shell}
      onPointerOverCapture={preloadLinkTarget}
      onFocusCapture={preloadLinkTarget}
    >
      <main ref={contentRef} className={styles.content}>
        {/* Suspense 只包内容区，不挂起左侧 Sidebar */}
        <Suspense fallback={<></>}>
          <Routes>
            <Route path={ROUTES.today} element={<TodayPage />} />
            <Route path={ROUTES.week} element={<WeekPage />} />
            <Route path={ROUTES.month} element={<MonthPage />} />
            <Route path={ROUTES.history} element={<HistoryPage />}>
              <Route index element={<HistoryStatsTab />} />
              <Route path="search" element={<SearchPage />} />
            </Route>
            <Route path={ROUTES.chat} element={<ChatPage />} />
            <Route path={ROUTES.aiSummary} element={<AISummaryPage />}>
              <Route index element={<DailyTab />} />
              <Route path="week" element={<WeeklyTab />} />
              <Route path="month" element={<MonthlyTab />} />
              <Route path="search" element={<Navigate to={`${ROUTES.history}/search`} replace />} />
              <Route path="debug" element={<Navigate to={ROUTES.aiSummary} replace />} />
            </Route>
            <Route path={ROUTES.aiSettings} element={<AISettingsPage />}>
              <Route index element={<Navigate to="models" replace />} />
              <Route path="models" element={<ModelsTab />} />
              <Route path="general" element={<AiGeneralTab />} />
              <Route path="prompt" element={<PromptTab />} />
              <Route path="external" element={<Navigate to={ROUTES.settingsAiModels} replace />} />
            </Route>
            <Route path={ROUTES.devices} element={<DevicesPage />} />
            <Route path={ROUTES.categories} element={<CategoriesPage />}>
              <Route index element={<ListTab />} />
              <Route path="apps" element={<AppsPage />} />
              <Route path="sites" element={<SitesPage />} />
            </Route>
            {/* 与主程序一致:旧的独立「应用」页并入分类页 tab 后仍可访问 */}
            <Route
              path={ROUTES.apps}
              element={<Navigate to={`${ROUTES.categories}/apps`} replace />}
            />
            <Route path={ROUTES.settings} element={<SettingsPage />}>
              <Route index element={<GeneralTab />} />
              <Route path="appearance" element={<AppearanceTab />} />
              <Route path="data" element={<DataTab />} />
              <Route path="privacy" element={<PrivacyTab />} />
              <Route path="models" element={<AiModelsTab />} />
              <Route path="engine" element={<Navigate to={ROUTES.settingsAiModels} replace />} />
              <Route path="plugins" element={<PluginsTab />} />
              <Route path="about" element={<AboutTab />} />
            </Route>
          </Routes>
        </Suspense>
      </main>
      <div className={styles.sidebarHost}>
        <Sidebar />
      </div>
      {MACOS_WINDOW && <TrafficLights />}
    </div>
  );
}

/** 极窄 iframe 阈值 —— 低于这个宽度才启用 CSS 缩放兜底（手机 / 平板竖屏）。
 *  桌面尺寸（>= 700px）让主应用响应式自适应，看着跟真原生应用尺寸一致。 */
const SCALE_BREAKPOINT = 700;
const DESIGN_WIDTH = 700;

/** 监听 iframe 实际宽。
 *  - >= 700px：scale = 1，应用直接按 iframe 真实尺寸渲染（响应式）
 *  - < 700px：scale = iframe宽 / 700，整体缩到不挤变形 */
function useFitScale(): { scale: number; designHeight: number } {
  const [size, setSize] = useState(() => {
    if (typeof window === "undefined") return { scale: 1, designHeight: 800 };
    const w = window.innerWidth;
    const scale = w < SCALE_BREAKPOINT ? w / DESIGN_WIDTH : 1;
    return { scale, designHeight: window.innerHeight / scale };
  });

  useEffect(() => {
    const update = () => {
      const w = window.innerWidth;
      const scale = w < SCALE_BREAKPOINT ? w / DESIGN_WIDTH : 1;
      const designHeight = window.innerHeight / scale;
      setSize({ scale, designHeight });
    };
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);

  return size;
}

export function AppDemo() {
  // 设置 body[data-platform]——主应用某些 CSS 按这个区分平台
  if (typeof document !== "undefined") {
    document.body.dataset.platform = MACOS_WINDOW ? "macos" : "windows";
  }

  const { scale, designHeight } = useFitScale();

  const providers = (
    <MemoryRouter initialEntries={["/"]}>
      <SettingsProvider>
        <UpdaterProvider>
          <CategoriesProvider>
            <SuperCategoriesProvider>
              <DeviceFilterProvider>
                <DemoLayout />
              </DeviceFilterProvider>
            </SuperCategoriesProvider>
          </CategoriesProvider>
        </UpdaterProvider>
      </SettingsProvider>
    </MemoryRouter>
  );

  // scale === 1 时直接渲染，避免无谓的 transform 层
  if (scale === 1) return providers;

  // 极窄 iframe：用 transform 缩放，保持应用按 700 设计宽度渲染再缩
  return (
    <div
      style={{
        width: `${DESIGN_WIDTH}px`,
        height: `${designHeight}px`,
        transform: `scale(${scale})`,
        transformOrigin: "top left",
        overflow: "hidden",
      }}
    >
      {providers}
    </div>
  );
}
