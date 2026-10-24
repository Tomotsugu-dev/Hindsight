// 主题库和正在用的自定义主题之间的同步：主题文件改了、删了、坏了，窗口跟着变。
import { themesApi, type InstalledTheme, type Theme } from "../api/themes";
import { logError } from "./logger";
import {
  customIdOf,
  getStoredCustomTheme,
  getStoredTheme,
  setCustomTheme,
  showFallbackTheme,
  type CustomTheme,
} from "./theme";

/** 读主题库，并按它更新正在用的自定义主题。返回主题库里的主题。 */
export async function loadThemes(): Promise<InstalledTheme[]> {
  const themes = await themesApi.list();
  await syncActiveTheme(themes);
  return themes;
}

/** 选中主题库里的主题 `theme`：读它的 theme.css，然后套用。 */
export async function chooseCustomTheme(theme: Theme): Promise<void> {
  const css = theme.hasCss ? await themesApi.css(theme.id) : null;
  setCustomTheme(toCustomTheme(theme, css));
}

/**
 * 启动时读一次主题库；之后窗口每次获得焦点再读，作者改完主题文件切回来就能看到。
 *
 * 选的是内置主题时不读：内置主题不需要主题库。
 */
export function watchThemeLibrary(): void {
  const sync = () => {
    if (customIdOf(getStoredTheme()) === null) return;
    loadThemes().catch((e) => logError("theme.sync", e));
  };
  sync();
  window.addEventListener("focus", sync);
}

/** 主题库里选中的主题能用就套用它的最新内容，用不了就先显示简约。 */
async function syncActiveTheme(themes: InstalledTheme[]): Promise<void> {
  const id = customIdOf(getStoredTheme());
  if (id === null) return;
  const theme = themes.find((t) => t.folderName === id)?.theme ?? null;
  let next: CustomTheme | null = null;
  if (theme !== null) {
    try {
      next = toCustomTheme(theme, theme.hasCss ? await themesApi.css(id) : null);
    } catch (e) {
      // 列出来以后 theme.css 被删了或改坏了：和主题坏了一样处理
      logError("theme.css", e);
    }
  }
  // 等后端的时候用户换了主题，就不再改
  if (customIdOf(getStoredTheme()) !== id) return;
  if (next === null) {
    showFallbackTheme();
  } else if (JSON.stringify(getStoredCustomTheme()) !== JSON.stringify(next)) {
    // 只在主题文件变了时重新套用，免得每次获得焦点图表都重画一遍
    setCustomTheme(next);
  }
}

function toCustomTheme(theme: Theme, css: string | null): CustomTheme {
  return { id: theme.id, layout: theme.layout, mode: theme.mode, tokens: theme.tokens, css };
}
