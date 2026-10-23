// App themes are a frontend-only preference stored in localStorage. The
// <html data-theme> attribute selects token overrides without backend settings.

export type AppTheme = "default" | "minimal" | "flat" | "dark";

/** Theme options rendered by the appearance picker. */
export const APP_THEMES: AppTheme[] = ["default", "minimal", "flat", "dark"];

const STORAGE_KEY = "hindsight.theme";

/** Read the saved theme; missing or invalid values use the default minimal theme. */
export function getStoredTheme(): AppTheme {
  const v = localStorage.getItem(STORAGE_KEY);
  if (v === "default" || v === "flat" || v === "dark") return v;
  return "minimal";
}

/** 把主题写到 <html data-theme>，让 tokens.css 的 [data-theme=...] 覆盖生效。 */
export function applyTheme(theme: AppTheme): void {
  document.documentElement.dataset.theme = theme;
}

/** 保存并立即应用。 */
export function setStoredTheme(theme: AppTheme): void {
  localStorage.setItem(STORAGE_KEY, theme);
  applyTheme(theme);
  listeners.forEach((l) => l());
}

// —— 主题变更订阅 ——
// 图表 / 图例这类组件把分类色注入 inline style（绕过 CSS 变量），切主题时得靠 JS
// 重新取色。这里给一个订阅点，配合 hooks/useTheme 的 useSyncExternalStore 让它们重渲。
const listeners = new Set<() => void>();

/** 订阅主题变化；返回取消订阅函数。 */
export function subscribeTheme(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** Read the active theme from `<html data-theme>`, falling back to minimal. */
export function getCurrentTheme(): AppTheme {
  const t = document.documentElement.dataset.theme;
  return t === "default" || t === "flat" || t === "dark" ? t : "minimal";
}
