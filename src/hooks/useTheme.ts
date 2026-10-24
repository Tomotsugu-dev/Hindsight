// 当前外观主题的 React 订阅。图表 / 图例等把分类色注入 inline style 的组件用它，
// 在切主题时重渲、按新主题重新取色（走 utils/categoryColor 的 adjustCategoryColor）。

import { useSyncExternalStore } from "react";
import { subscribeTheme, getCurrentMode, getCurrentTheme, type AppTheme } from "../lib/theme";

/** 订阅并返回当前主题。 */
export function useTheme(): AppTheme {
  return useSyncExternalStore(
    subscribeTheme,
    getCurrentTheme,
    getCurrentTheme,
  );
}

const isDarkNow = () => getCurrentMode() === "dark";

/** 便捷：当前是否暗色。读 data-mode，不看主题名。 */
export function useIsDark(): boolean {
  return useSyncExternalStore(subscribeTheme, isDarkNow, isDarkNow);
}
