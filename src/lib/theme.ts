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

/** 选中的主题：内置主题名，或 `custom:<主题 id>`。 */
export type ThemeChoice = AppTheme | `custom:${string}`;

/** 正在用的自定义主题里，套用时要用的部分。 */
export interface CustomTheme {
  id: string;
  layout: PageLayout;
  mode: ColorMode;
  /** 公开变量：变量名 → 值 */
  tokens: Record<string, string>;
  /** theme.css 的内容；没有时是 null */
  css: string | null;
}

/** 每个内置主题用的布局和明暗。 */
const BUILT_IN_LOOKS: Record<AppTheme, { layout: PageLayout; mode: ColorMode }> = {
  default: { layout: "card", mode: "light" },
  minimal: { layout: "card", mode: "light" },
  flat: { layout: "flat", mode: "light" },
  dark: { layout: "card", mode: "dark" },
};

const STORAGE_KEY = "hindsight.theme";

/** 正在用的自定义主题的副本。启动时直接用它，不等后端读主题文件。 */
const CUSTOM_KEY = "hindsight.theme.custom";

const CUSTOM_PREFIX = "custom:";

/** 自定义主题 `id` 的选择。 */
export function customChoice(id: string): ThemeChoice {
  return `${CUSTOM_PREFIX}${id}`;
}

/** 选的自定义主题的 id；选的是内置主题时返回 null。 */
export function customIdOf(choice: ThemeChoice): string | null {
  return choice.startsWith(CUSTOM_PREFIX) ? choice.slice(CUSTOM_PREFIX.length) : null;
}

/** Read the saved theme; missing or invalid values use the default minimal theme. */
export function getStoredTheme(): ThemeChoice {
  const v = localStorage.getItem(STORAGE_KEY);
  if (v === "default" || v === "flat" || v === "dark") return v;
  if (v !== null && v.length > CUSTOM_PREFIX.length && v.startsWith(CUSTOM_PREFIX)) {
    return v as ThemeChoice;
  }
  return "minimal";
}

/** 读自定义主题的副本；没有或格式不对时返回 null。 */
export function getStoredCustomTheme(): CustomTheme | null {
  const raw = localStorage.getItem(CUSTOM_KEY);
  if (raw === null) return null;
  try {
    const v: unknown = JSON.parse(raw);
    return isCustomTheme(v) ? v : null;
  } catch {
    return null;
  }
}

function isCustomTheme(v: unknown): v is CustomTheme {
  if (typeof v !== "object" || v === null) return false;
  const t = v as Record<string, unknown>;
  return (
    typeof t.id === "string" &&
    (t.layout === "card" || t.layout === "flat") &&
    (t.mode === "light" || t.mode === "dark") &&
    typeof t.tokens === "object" &&
    t.tokens !== null &&
    Object.values(t.tokens).every((value) => typeof value === "string") &&
    (t.css === null || typeof t.css === "string")
  );
}

// 正在用的自定义主题写到页面上的东西，换主题时要撤掉
let appliedTokens: string[] = [];
let appliedSheet: CSSStyleSheet | null = null;

function clearCustomStyles(): void {
  const { style } = document.documentElement;
  appliedTokens.forEach((name) => style.removeProperty(name));
  appliedTokens = [];
  if (appliedSheet) {
    const sheet = appliedSheet;
    document.adoptedStyleSheets = document.adoptedStyleSheets.filter((s) => s !== sheet);
    appliedSheet = null;
  }
}

/** 把主题写到 <html> 的 data-theme、data-layout、data-mode 上，tokens.css 和组件样式按它们生效。 */
export function applyTheme(theme: AppTheme): void {
  clearCustomStyles();
  const { dataset } = document.documentElement;
  const { layout, mode } = BUILT_IN_LOOKS[theme];
  dataset.theme = theme;
  dataset.layout = layout;
  dataset.mode = mode;
}

/**
 * 套用自定义主题：配色从默认主题开始（`data-theme="custom"` 没有自己的配色），变量写在
 * <html> 的 style 上，theme.css 加在所有样式表之后。所以变量压过 theme.css，theme.css 压过内置样式。
 */
export function applyCustomTheme(theme: CustomTheme): void {
  clearCustomStyles();
  const root = document.documentElement;
  root.dataset.theme = "custom";
  root.dataset.layout = theme.layout;
  root.dataset.mode = theme.mode;
  for (const [name, value] of Object.entries(theme.tokens)) {
    root.style.setProperty(name, value);
  }
  appliedTokens = Object.keys(theme.tokens);
  if (theme.css !== null) {
    // adoptedStyleSheets 排在页面所有 <style>、<link> 之后，后加载的页面样式也排在它前面
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(theme.css);
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
    appliedSheet = sheet;
  }
}

/** 保存并立即应用。 */
export function setStoredTheme(theme: AppTheme): void {
  localStorage.setItem(STORAGE_KEY, theme);
  localStorage.removeItem(CUSTOM_KEY);
  applyTheme(theme);
  notify();
}

/** 选中自定义主题：保存选择和副本，立即应用。 */
export function setCustomTheme(theme: CustomTheme): void {
  localStorage.setItem(STORAGE_KEY, customChoice(theme.id));
  try {
    localStorage.setItem(CUSTOM_KEY, JSON.stringify(theme));
  } catch {
    // 存不下（theme.css 很大）：不留旧副本，启动时先显示简约，读完主题库再换
    localStorage.removeItem(CUSTOM_KEY);
  }
  applyCustomTheme(theme);
  notify();
}

/**
 * 选中的自定义主题用不了：保留选择，删掉副本，先显示简约。
 *
 * 主题修好或放回来后，下一次读主题库就会换回去。
 */
export function showFallbackTheme(): void {
  localStorage.removeItem(CUSTOM_KEY);
  // 已经是简约时不再通知，免得每次获得焦点图表都重画一遍
  if (document.documentElement.dataset.theme !== "custom") return;
  applyTheme("minimal");
  notify();
}

/** 启动时套用保存的主题。选的是自定义主题、但副本不是它时，先显示简约，等读完主题库再换。 */
export function restoreTheme(): void {
  const choice = getStoredTheme();
  const id = customIdOf(choice);
  if (id === null) {
    applyTheme(choice as AppTheme);
    return;
  }
  const saved = getStoredCustomTheme();
  if (saved?.id === id) {
    applyCustomTheme(saved);
  } else {
    localStorage.removeItem(CUSTOM_KEY);
    applyTheme("minimal");
  }
}

// —— 主题变更订阅 ——
// 图表 / 图例这类组件把分类色注入 inline style（绕过 CSS 变量），切主题时得靠 JS
// 重新取色。这里给一个订阅点，配合 hooks/useTheme 的 useSyncExternalStore 让它们重渲。
const listeners = new Set<() => void>();

function notify(): void {
  listeners.forEach((l) => l());
}

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
