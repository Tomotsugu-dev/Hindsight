// Demo entry. 比主 main.tsx 简单：
// - 不挂 SettingsProvider / CategoriesProvider 等（Stage 1 hello world 阶段）
// - Stage 3 以后会逐步把 providers 都挂上
// - 注入 Tauri internals 全局 stub，防止某些组件运行时报错

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

import React from "react";
import ReactDOM from "react-dom/client";
import { applyTheme, getStoredTheme } from "@app/lib/theme";
import { AppDemo } from "./AppDemo";
import "./mobile-overrides.css";

// 跟主 main.tsx 一致:启动即应用主题(默认 minimal 简约),
// 否则 <html> 无 data-theme,会渲染成 default 多彩主题
applyTheme(getStoredTheme());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppDemo />
  </React.StrictMode>,
);
