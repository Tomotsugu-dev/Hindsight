// App themes are a frontend-only preference stored in localStorage. Three
// attributes on <html> select token overrides without backend settings:
// data-theme picks the palette, data-layout and data-mode pick the look.

export type AppTheme = "default" | "minimal" | "flat" | "dark";

/** Theme options rendered by the appearance picker. */
export const APP_THEMES: AppTheme[] = ["default", "minimal", "flat", "dark"];

/** 页面布局：卡片还是扁平。取值和后端主题文件的 `layout` 一样。 */
export type PageLayout = "card" | "flat";

/** 配色的明暗。取值和后端主题文件的 `mode` 一样。 */
export type ColorMode = "light" | "dark";

/** 每个内置主题用的布局和明暗。 */
const BUILT_IN_LOOKS: Record<AppTheme, { layout: PageLayout; mode: ColorMode }> = {
  default: { layout: "card", mode: "light" },
  minimal: { layout: "card", mode: "light" },
  flat: { layout: "flat", mode: "light" },
  dark: { layout: "card", mode: "dark" },
};

const STORAGE_KEY = "hindsight.theme";

/** Read the saved theme; missing or invalid values use the default minimal theme. */
export function getStoredTheme(): AppTheme {
  const v = localStorage.getItem(STORAGE_KEY);
  if (v === "default" || v === "flat" || v === "dark") return v;
  return "minimal";
}

/** 把主题写到 <html> 的 data-theme、data-layout、data-mode 上，tokens.css 和组件样式按它们生效。 */
export function applyTheme(theme: AppTheme): void {
  const { dataset } = document.documentElement;
  const { layout, mode } = BUILT_IN_LOOKS[theme];
  dataset.theme = theme;
  dataset.layout = layout;
  dataset.mode = mode;
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

/** 从 `<html data-mode>` 读当前的明暗；不看主题名，没设时当亮色。 */
export function getCurrentMode(): ColorMode {
  return document.documentElement.dataset.mode === "dark" ? "dark" : "light";
}
