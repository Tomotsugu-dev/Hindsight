import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { type } from "@tauri-apps/plugin-os";
import App from "./App";
import { DeviceFilterProvider } from "./state/deviceFilter";
import { CategoriesProvider } from "./state/categories";
import { SettingsProvider } from "./state/settings";
import { UpdaterProvider } from "./state/updater";
import "./i18n";
import "./styles/global.css";

// 把当前 OS 写到 body[data-platform]，CSS 据此区分 macOS / windows / linux 的 chrome 表现
try {
  document.body.dataset.platform = type();
} catch {
  document.body.dataset.platform = "windows"; // 默认按 Windows 渲染
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <BrowserRouter>
      <SettingsProvider>
        <UpdaterProvider>
          <CategoriesProvider>
            <DeviceFilterProvider>
              <App />
            </DeviceFilterProvider>
          </CategoriesProvider>
        </UpdaterProvider>
      </SettingsProvider>
    </BrowserRouter>
  </React.StrictMode>,
);
