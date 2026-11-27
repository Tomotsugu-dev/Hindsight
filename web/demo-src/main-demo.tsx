// Demo entry.
// - 注入 Tauri internals 全局 stub，防止某些组件运行时报错
// - 启动时并行拉取当前语言包、首屏 TodayPage chunk，以及（若有）?theme= 预览主题
// - 支持 /themes/ 页面的 ?theme-host=1 常驻预览宿主模式，通过同源 postMessage 瞬时切主题

// 注入 Tauri internals stub——某些组件可能直接访问 window.__TAURI_INTERNALS__
declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      invoke?: (...args: unknown[]) => Promise<unknown>;
    };
  }
}

if (typeof window !== "undefined" && !window.__TAURI_INTERNALS__) {
  window.__TAURI_INTERNALS__ = {
    invoke: async () => undefined,
  };
}

// 必须是第一个 import：要在 i18n 启动前设好语言
import "./lang-from-query";
// Before any page module: some read the UI state as they load
import "./ui-state-demo";
import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import { ErrorBoundary } from "@app/components/ErrorBoundary/ErrorBoundary";
import { restoreTheme } from "@app/lib/theme";
import { AppDemo } from "./AppDemo";
import { ensureInitialLocale } from "./i18n-demo";
import { PreviewError } from "./PreviewError";
import { applyPreviewTheme, installPreviewThemeHost, isThemeHostMode } from "./preview-theme";
import { followSiteMode } from "./site-mode";
import "./mobile-overrides.css";

// 跟主 main.tsx 一致:启动即应用主题(默认 minimal 简约),
// 否则 <html> 无 data-theme,会渲染成 default 多彩主题
restoreTheme();

const params = new URLSearchParams(location.search);
const isThemePreview = params.has("theme") || isThemeHostMode(location.search);

// 主题页的预览要显示指定主题，不跟官网的深浅色
if (!isThemePreview) followSiteMode();

function DemoRoot({ initialFailed, hostMode }: { initialFailed: boolean; hostMode: boolean }) {
  const [failed, setFailed] = useState(initialFailed);

  useEffect(() => {
    if (!hostMode) return;
    return installPreviewThemeHost(setFailed);
  }, [hostMode]);

  return (
    <ErrorBoundary scope="demo.crash">
      {failed && <PreviewError />}
      <AppDemo />
    </ErrorBoundary>
  );
}

// 并行加载当前语言包 +（若有）预览主题，都准备好再首次渲染，避免闪出 key 或默认主题
Promise.all([ensureInitialLocale(), applyPreviewTheme(location.search)]).then(([, failed]) => {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <DemoRoot initialFailed={failed} hostMode={isThemeHostMode(location.search)} />
    </React.StrictMode>,
  );
});
