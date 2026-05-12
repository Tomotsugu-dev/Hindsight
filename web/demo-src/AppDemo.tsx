// Demo 根组件 —— 接入真 Sidebar + Router + 主应用页面。
// 跟主 main.tsx 的差异：
// 1. 用 MemoryRouter 而不是 BrowserRouter（iframe 内 URL bar 无意义，避免 base path 问题）
// 2. 不挂 WindowControls（iframe 内无窗口控件需要）
// 3. data-platform 强制 "windows"

import { lazy, Suspense, useEffect, useState } from "react";
import { MemoryRouter, Routes, Route } from "react-router-dom";

import { SettingsProvider } from "@app/state/settings";
import { UpdaterProvider } from "@app/state/updater";
import { CategoriesProvider } from "@app/state/categories";
import { DeviceFilterProvider } from "@app/state/deviceFilter";
import { Sidebar } from "@app/components/Sidebar/Sidebar";
import { ROUTES } from "@app/config/nav";

import "@app/i18n";
import "@app/styles/global.css";
import "@app/styles/tokens.css";
import "@app/styles/utilities.css";

import styles from "./AppDemo.module.css";

// Stage 4: 全部 8 个 sidebar 项接入真页面
const TodayPage = lazy(() => import("@app/pages/Today/TodayPage"));
const WeekPage = lazy(() => import("@app/pages/Week/WeekPage"));
const MonthPage = lazy(() => import("@app/pages/Month/MonthPage"));
const AISummaryPage = lazy(() => import("@app/pages/AISummary/AISummaryPage"));
const DailyTab = lazy(() => import("@app/pages/AISummary/tabs/DailyTab"));
const WeeklyTab = lazy(() => import("@app/pages/AISummary/tabs/WeeklyTab"));
const MonthlyTab = lazy(() => import("@app/pages/AISummary/tabs/MonthlyTab"));
const ChatTab = lazy(() => import("@app/pages/AISummary/tabs/ChatTab"));
const DebugTab = lazy(() => import("@app/pages/AISummary/tabs/DebugTab"));
const AISettingsPage = lazy(() => import("@app/pages/AISettings/AISettingsPage"));
const EngineTab = lazy(() => import("@app/pages/AISettings/tabs/EngineTab"));
const ModelsTab = lazy(() => import("@app/pages/AISettings/tabs/ModelsTab"));
const AiGeneralTab = lazy(() => import("@app/pages/AISettings/tabs/GeneralTab"));
const PromptTab = lazy(() => import("@app/pages/AISettings/tabs/PromptTab"));
const ExternalApiTab = lazy(() => import("@app/pages/AISettings/tabs/ExternalApiTab"));
const DevicesPage = lazy(() => import("@app/pages/Devices/DevicesPage"));
const CategoriesPage = lazy(() => import("@app/pages/Categories/CategoriesPage"));
const SettingsPage = lazy(() => import("@app/pages/Settings/SettingsPage"));
const GeneralTab = lazy(() => import("@app/pages/Settings/tabs/GeneralTab"));
const DataTab = lazy(() => import("@app/pages/Settings/tabs/DataTab"));
const PrivacyTab = lazy(() => import("@app/pages/Settings/tabs/PrivacyTab"));
const AboutTab = lazy(() => import("@app/pages/Settings/tabs/AboutTab"));

function DemoLayout() {
  return (
    <div className={styles.shell}>
      <main className={styles.content}>
        <Routes>
          <Route path={ROUTES.today} element={<TodayPage />} />
          <Route path={ROUTES.week} element={<WeekPage />} />
          <Route path={ROUTES.month} element={<MonthPage />} />
          <Route path={ROUTES.aiSummary} element={<AISummaryPage />}>
            <Route index element={<DailyTab />} />
            <Route path="week" element={<WeeklyTab />} />
            <Route path="month" element={<MonthlyTab />} />
            <Route path="chat" element={<ChatTab />} />
            <Route path="debug" element={<DebugTab />} />
          </Route>
          <Route path={ROUTES.aiSettings} element={<AISettingsPage />}>
            <Route index element={<EngineTab />} />
            <Route path="models" element={<ModelsTab />} />
            <Route path="general" element={<AiGeneralTab />} />
            <Route path="prompt" element={<PromptTab />} />
            <Route path="external" element={<ExternalApiTab />} />
          </Route>
          <Route path={ROUTES.devices} element={<DevicesPage />} />
          <Route path={ROUTES.categories} element={<CategoriesPage />} />
          <Route path={ROUTES.settings} element={<SettingsPage />}>
            <Route index element={<GeneralTab />} />
            <Route path="data" element={<DataTab />} />
            <Route path="privacy" element={<PrivacyTab />} />
            <Route path="about" element={<AboutTab />} />
          </Route>
        </Routes>
      </main>
      <div className={styles.sidebarHost}>
        <Sidebar />
      </div>
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
    document.body.dataset.platform = "windows";
  }

  const { scale, designHeight } = useFitScale();

  // scale === 1 时直接渲染，避免无谓的 transform 层
  if (scale === 1) {
    return (
      <MemoryRouter initialEntries={["/"]}>
        <SettingsProvider>
          <UpdaterProvider>
            <CategoriesProvider>
              <DeviceFilterProvider>
                <Suspense fallback={<></>}>
                  <DemoLayout />
                </Suspense>
              </DeviceFilterProvider>
            </CategoriesProvider>
          </UpdaterProvider>
        </SettingsProvider>
      </MemoryRouter>
    );
  }

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
      <MemoryRouter initialEntries={["/"]}>
        <SettingsProvider>
          <UpdaterProvider>
            <CategoriesProvider>
              <DeviceFilterProvider>
                <Suspense fallback={<></>}>
                  <DemoLayout />
                </Suspense>
              </DeviceFilterProvider>
            </CategoriesProvider>
          </UpdaterProvider>
        </SettingsProvider>
      </MemoryRouter>
    </div>
  );
}
