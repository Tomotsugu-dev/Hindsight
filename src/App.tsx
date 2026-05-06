import { useEffect } from "react";
import { Routes, Route } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { AppLayout } from "./layouts/AppLayout";
import { ROUTES } from "./config/nav";
import { useSettings } from "./state/settings";
import type { PromptLanguage } from "./api/hindsight";
import Today from "./pages/Today/TodayPage";
import Week from "./pages/Week/WeekPage";
import Month from "./pages/Month/MonthPage";
import AISummaryPage from "./pages/AISummary/AISummaryPage";
import DailyTab from "./pages/AISummary/tabs/DailyTab";
import WeeklyTab from "./pages/AISummary/tabs/WeeklyTab";
import MonthlyTab from "./pages/AISummary/tabs/MonthlyTab";
import ChatTab from "./pages/AISummary/tabs/ChatTab";
import DebugTab from "./pages/AISummary/tabs/DebugTab";
import AISettingsPage from "./pages/AISettings/AISettingsPage";
import EngineTab from "./pages/AISettings/tabs/EngineTab";
import AiGeneralTab from "./pages/AISettings/tabs/GeneralTab";
import PromptTab from "./pages/AISettings/tabs/PromptTab";
import ExternalApiTab from "./pages/AISettings/tabs/ExternalApiTab";
import Devices from "./pages/Devices/DevicesPage";
import CategoriesPage from "./pages/Categories/CategoriesPage";
import SettingsPage from "./pages/Settings/SettingsPage";
import GeneralTab from "./pages/Settings/tabs/GeneralTab";
import DataTab from "./pages/Settings/tabs/DataTab";
import PrivacyTab from "./pages/Settings/tabs/PrivacyTab";
import AboutTab from "./pages/Settings/tabs/AboutTab";

/** 把 i18n 当前语言映射到 settings.ai.promptLanguage 的取值（zh/en/ja）。 */
function i18nToPromptLang(lang: string): PromptLanguage {
  if (lang.startsWith("en")) return "en";
  if (lang.startsWith("ja")) return "ja";
  return "zh";
}

function App() {
  const { i18n } = useTranslation();
  const { settings, update } = useSettings();

  // UI 语言切换时同步 settings.ai.promptLanguage —— 让 AISettings 提示词编辑器、
  // DebugTab、以及后端 generate 用的 prompt 都跟随 UI 语言走
  useEffect(() => {
    if (!settings) return;
    const target = i18nToPromptLang(i18n.language);
    if (settings.ai.promptLanguage !== target) {
      update({ ai: { ...settings.ai, promptLanguage: target } });
    }
  }, [i18n.language, settings, update]);

  return (
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
          <Route path="general" element={<AiGeneralTab />} />
          <Route path="prompt" element={<PromptTab />} />
          <Route path="external" element={<ExternalApiTab />} />
        </Route>
        <Route path={ROUTES.devices} element={<Devices />} />
        <Route path={ROUTES.categories} element={<CategoriesPage />} />
        <Route path={ROUTES.settings} element={<SettingsPage />}>
          <Route index element={<GeneralTab />} />
          <Route path="data" element={<DataTab />} />
          <Route path="privacy" element={<PrivacyTab />} />
          <Route path="about" element={<AboutTab />} />
        </Route>
      </Route>
    </Routes>
  );
}

export default App;
