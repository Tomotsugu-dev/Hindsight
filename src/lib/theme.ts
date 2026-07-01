// 应用外观主题。纯前端偏好：存 localStorage，通过 <html data-theme> 驱动
// styles/tokens.css 里的 [data-theme=...] 变量覆盖，不经过后端 settings。

export type AppTheme = "default" | "minimal" | "dark";

/** 三种可选主题，供 UI 遍历渲染选项。 */
export const APP_THEMES: AppTheme[] = ["default", "minimal", "dark"];

const STORAGE_KEY = "hindsight.theme";

/** 读已保存的主题；无 / 非法值回退 minimal（简约为应用默认外观）。 */
export function getStoredTheme(): AppTheme {
  const v = localStorage.getItem(STORAGE_KEY);
  if (v === "default" || v === "dark") return v;
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

/** 读当前生效主题（从 `<html data-theme>`，非法/缺省回退 minimal）。 */
export function getCurrentTheme(): AppTheme {
  const t = document.documentElement.dataset.theme;
  return t === "default" || t === "dark" ? t : "minimal";
}
