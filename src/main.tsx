import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { type } from "@tauri-apps/plugin-os";
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary/ErrorBoundary";
import { DeviceFilterProvider } from "./state/deviceFilter";
import { CategoriesProvider } from "./state/categories";
import { SuperCategoriesProvider } from "./state/superCategories";
import { SettingsProvider } from "./state/settings";
import { UpdaterProvider } from "./state/updater";
import { ensureInitialLocale } from "./i18n";
import { applyTheme, getStoredTheme } from "./lib/theme";
import "./styles/global.css";

// 把当前 OS 写到 body[data-platform]，CSS 据此区分 macOS / windows / linux 的 chrome 表现
try {
  document.body.dataset.platform = type();
} catch {
  document.body.dataset.platform = "windows"; // 默认按 Windows 渲染
}

// 渲染前先套用已保存的外观主题，避免首帧闪一下默认亮色再切到简约 / 暗色
applyTheme(getStoredTheme());

// 首启按系统语言设好 locale 再渲染，避免闪一下兜底语言；有显式选择则瞬时 resolve
void ensureInitialLocale().finally(() => {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      {/* 顶层边界：任何 provider / 路由层抛错都兜在这里，避免整窗白屏 */}
      <ErrorBoundary scope="app.crash">
        <BrowserRouter>
          <SettingsProvider>
            <UpdaterProvider>
              <CategoriesProvider>
                {/* SuperCategoriesProvider 依赖 useCategories，必须嵌套在 CategoriesProvider 内 */}
                <SuperCategoriesProvider>
                  <DeviceFilterProvider>
                    <App />
                  </DeviceFilterProvider>
                </SuperCategoriesProvider>
              </CategoriesProvider>
            </UpdaterProvider>
          </SettingsProvider>
        </BrowserRouter>
      </ErrorBoundary>
    </React.StrictMode>,
  );
});
