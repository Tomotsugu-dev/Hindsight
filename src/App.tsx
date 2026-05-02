import { Routes, Route } from "react-router-dom";
import { AppLayout } from "./layouts/AppLayout";
import { ROUTES } from "./config/nav";
import Today from "./pages/Today";
import Week from "./pages/Week";
import Month from "./pages/Month";
import Sync from "./pages/Sync";
import Settings from "./pages/Settings";

function App() {
  return (
    <Routes>
      <Route element={<AppLayout />}>
        <Route path={ROUTES.today} element={<Today />} />
        <Route path={ROUTES.week} element={<Week />} />
        <Route path={ROUTES.month} element={<Month />} />
        <Route path={ROUTES.sync} element={<Sync />} />
        <Route path={ROUTES.settings} element={<Settings />} />
      </Route>
    </Routes>
  );
}

export default App;
