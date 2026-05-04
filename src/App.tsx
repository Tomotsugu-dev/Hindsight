import { Routes, Route } from "react-router-dom";
import { AppLayout } from "./layouts/AppLayout";
import { ROUTES } from "./config/nav";
import Today from "./pages/Today/TodayPage";
import Week from "./pages/Week/WeekPage";
import Month from "./pages/Month/MonthPage";
import AISummary from "./pages/AISummary";
import AISettings from "./pages/AISettings";
import Devices from "./pages/Devices/DevicesPage";
import CategoriesPage from "./pages/Categories/CategoriesPage";
import SettingsPage from "./pages/Settings/SettingsPage";
import GeneralTab from "./pages/Settings/tabs/GeneralTab";
import DataTab from "./pages/Settings/tabs/DataTab";
import PrivacyTab from "./pages/Settings/tabs/PrivacyTab";
import AboutTab from "./pages/Settings/tabs/AboutTab";

function App() {
  return (
    <Routes>
      <Route element={<AppLayout />}>
        <Route path={ROUTES.today} element={<Today />} />
        <Route path={ROUTES.week} element={<Week />} />
        <Route path={ROUTES.month} element={<Month />} />
        <Route path={ROUTES.aiSummary} element={<AISummary />} />
        <Route path={ROUTES.aiSettings} element={<AISettings />} />
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
