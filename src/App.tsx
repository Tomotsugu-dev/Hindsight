import { Routes, Route } from "react-router-dom";
import { AppLayout } from "./layouts/AppLayout";
import { ROUTES } from "./config/nav";
import Today from "./pages/Today/TodayPage";
import Week from "./pages/Week";
import Month from "./pages/Month";
import AI from "./pages/AI";
import Sync from "./pages/Sync";
import SettingsPage from "./pages/Settings/SettingsPage";
import GeneralTab from "./pages/Settings/tabs/GeneralTab";
import DataTab from "./pages/Settings/tabs/DataTab";
import LabelsTab from "./pages/Settings/tabs/LabelsTab";
import AboutTab from "./pages/Settings/tabs/AboutTab";

function App() {
  return (
    <Routes>
      <Route element={<AppLayout />}>
        <Route path={ROUTES.today} element={<Today />} />
        <Route path={ROUTES.week} element={<Week />} />
        <Route path={ROUTES.month} element={<Month />} />
        <Route path={ROUTES.ai} element={<AI />} />
        <Route path={ROUTES.sync} element={<Sync />} />
        <Route path={ROUTES.settings} element={<SettingsPage />}>
          <Route index element={<GeneralTab />} />
          <Route path="data" element={<DataTab />} />
          <Route path="labels" element={<LabelsTab />} />
          <Route path="about" element={<AboutTab />} />
        </Route>
      </Route>
    </Routes>
  );
}

export default App;
