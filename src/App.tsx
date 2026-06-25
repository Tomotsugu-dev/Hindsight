import { lazy, Suspense, useEffect } from "react";
import { Routes, Route } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { AppLayout } from "./layouts/AppLayout";
import { ROUTES } from "./config/nav";
import { useSettings } from "./state/settings";
import { api, type PromptLanguage } from "./api/hindsight";
import {
  isDefaultSegments,
  retranslateDefaultSegments,
} from "./utils/defaultSegments";

// 代码拆分：所有 page / tab 都走 `React.lazy`，Vite 给每个组件出独立 chunk。
// 首屏只加载 sidebar / layout / `i18n` 这些必备的东西；切到某个 tab 时按需 fetch。
// 同一 page 内 tab 之间切换有可能闪一下 fallback，但每个 chunk 都很小，浏览器
// HTTP/2 多路复用下基本看不到。
const Today = lazy(() => import("./pages/Today/TodayPage"));
const Week = lazy(() => import("./pages/Week/WeekPage"));
const Month = lazy(() => import("./pages/Month/MonthPage"));
const AISummaryPage = lazy(() => import("./pages/AISummary/AISummaryPage"));
const DailyTab = lazy(() => import("./pages/AISummary/tabs/DailyTab"));
const WeeklyTab = lazy(() => import("./pages/AISummary/tabs/WeeklyTab"));
const MonthlyTab = lazy(() => import("./pages/AISummary/tabs/MonthlyTab"));
const ChatTab = lazy(() => import("./pages/AISummary/tabs/ChatTab"));
const DebugTab = lazy(() => import("./pages/AISummary/tabs/DebugTab"));
const AISettingsPage = lazy(() => import("./pages/AISettings/AISettingsPage"));
const EngineTab = lazy(() => import("./pages/AISettings/tabs/EngineTab"));
const ModelsTab = lazy(() => import("./pages/AISettings/tabs/ModelsTab"));
const AiGeneralTab = lazy(() => import("./pages/AISettings/tabs/GeneralTab"));
const PromptTab = lazy(() => import("./pages/AISettings/tabs/PromptTab"));
const ExternalApiTab = lazy(() => import("./pages/AISettings/tabs/ExternalApiTab"));
const Devices = lazy(() => import("./pages/Devices/DevicesPage"));
const CategoriesPage = lazy(() => import("./pages/Categories/CategoriesPage"));
const AppsPage = lazy(() => import("./pages/Apps/AppsPage"));
const SettingsPage = lazy(() => import("./pages/Settings/SettingsPage"));
const GeneralTab = lazy(() => import("./pages/Settings/tabs/GeneralTab"));
const DataTab = lazy(() => import("./pages/Settings/tabs/DataTab"));
const PrivacyTab = lazy(() => import("./pages/Settings/tabs/PrivacyTab"));
const AboutTab = lazy(() => import("./pages/Settings/tabs/AboutTab"));

/** 把 i18n 当前语言映射到 settings.ai.promptLanguage 的取值（zh/en/ja/pt）。 */
function i18nToPromptLang(lang: string): PromptLanguage {
  if (lang.startsWith("en")) return "en";
  if (lang.startsWith("ja")) return "ja";
  if (lang.startsWith("pt")) return "pt";
  return "zh";
}

function App() {
  const { t, i18n } = useTranslation();
  const { settings, update } = useSettings();

  // UI 语言切换时同步 AI 设置：
  //  1. promptLanguage —— 让 AISettings 提示词编辑器、DebugTab、后端 generate 的 prompt 都跟随 UI 语言
  //  2. 时段标签若仍是某语言的默认 → 跟着重译成新语言的默认（用户自定义过则不动，颜色保留）
  useEffect(() => {
    if (!settings) return;
    const nextAi = { ...settings.ai };
    let changed = false;

    const promptLang = i18nToPromptLang(i18n.language);
    if (settings.ai.promptLanguage !== promptLang) {
      nextAi.promptLanguage = promptLang;
      changed = true;
    }

    if (isDefaultSegments(settings.ai.segments)) {
      const reseg = retranslateDefaultSegments(settings.ai.segments, i18n.language);
      if (reseg.some((s, i) => s.label !== settings.ai.segments[i].label)) {
        nextAi.segments = reseg;
        changed = true;
      }
    }

    if (changed) update({ ai: nextAi });
  }, [i18n.language, settings, update]);

  // 原生托盘菜单不走前端 i18n，挂载 + 切语言时把译文推给后端 set_tray_labels 同步
  useEffect(() => {
    void api.setTrayLabels(t("tray.show"), t("tray.quit")).catch(() => {});
  }, [t, i18n.language]);

  return (
    // Suspense fallback 故意保持空 —— page chunk 通常 < 50KB，本地加载几十毫秒级，
    // 闪 spinner 反而扰人。如果将来某个 page 体积涨到肉眼可感的程度（数百 KB+），
    // 再换成全屏 skeleton / spinner。
    <Suspense fallback={<></>}>
      <Routes>
        <Route element={<AppLayout />}>
          <Route path={ROUTES.today} element={<Today />} />
          <Route path={ROUTES.week} element={<Week />} />
          <Route path={ROUTES.month} element={<Month />} />
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
          <Route path={ROUTES.devices} element={<Devices />} />
          <Route path={ROUTES.categories} element={<CategoriesPage />} />
          <Route path={ROUTES.apps} element={<AppsPage />} />
          <Route path={ROUTES.settings} element={<SettingsPage />}>
            <Route index element={<GeneralTab />} />
            <Route path="data" element={<DataTab />} />
            <Route path="privacy" element={<PrivacyTab />} />
            <Route path="about" element={<AboutTab />} />
          </Route>
        </Route>
      </Routes>
    </Suspense>
  );
}

export default App;
