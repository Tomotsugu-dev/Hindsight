// Renders the main window. main.tsx imports this file after the UI state is loaded.
import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary/ErrorBoundary";
import { DeviceFilterProvider } from "./state/deviceFilter";
import { CategoriesProvider } from "./state/categories";
import { SuperCategoriesProvider } from "./state/superCategories";
import { SettingsProvider } from "./state/settings";
import { UpdaterProvider } from "./state/updater";
import { ensureInitialLocale } from "./i18n";
import { translateDefaultCategoryNames } from "./utils/categoryName";
import { restoreTheme } from "./lib/theme";
import { watchThemeLibrary } from "./lib/themeLibrary";
import "./styles/global.css";

// 渲染前先套用已保存的外观主题，避免首帧闪一下默认亮色再切到简约 / 暗色。
// 自定义主题先用保存的副本，再读主题库拿最新的
restoreTheme();
watchThemeLibrary();

// 首启按系统语言设好 locale 再渲染，避免闪一下兜底语言；有显式选择则瞬时 resolve。
// 默认分类名按这个语言换好再渲染，分类列表第一次读到的就是译名
void ensureInitialLocale()
  .then(translateDefaultCategoryNames)
  .finally(() => {
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
