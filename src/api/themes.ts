// 自定义主题的命令和数据。类型对应 src-tauri/src/themes/plugin/ 和 commands/themes.rs。
import { invoke } from "@tauri-apps/api/core";
import type { ColorMode, PageLayout } from "../lib/theme";

/** 检查过的 theme.json。 */
export interface Theme {
  id: string;
  name: string;
  version: string;
  /** 没写时是空串 */
  author: string;
  /** 各种语言的简介：语言代码 → 文字。用 pickDescription 按界面语言挑一种 */
  description: Record<string, string>;
  layout: PageLayout;
  mode: ColorMode;
  /** 公开变量：变量名 → 值 */
  tokens: Record<string, string>;
  /** theme.json 里不在公开名单上、没有生效的变量，按名字排 */
  ignoredTokens: string[];
  hasCss: boolean;
}

/** 主题库里的一个文件夹。读出来了 `theme` 有值；读不出来 `error` 写着原因。 */
export interface InstalledTheme {
  folder: string;
  theme: Theme | null;
  error: string | null;
}

export const themesApi = {
  list: () => invoke<InstalledTheme[]>("theme_list"),
  /** 主题的 theme.css；没有时是 null */
  css: (id: string) => invoke<string | null>("theme_css", { id }),
  /** 导入用户选的 theme.json，同一文件夹里的 theme.css 一起导入；已有同 id 的主题时替换它 */
  import: (path: string) => invoke<Theme>("theme_import", { path }),
  /** 按文件夹名删，坏掉的主题也能删 */
  delete: (folder: string) => invoke<void>("theme_delete", { folder }),
  openFolder: () => invoke<void>("theme_open_folder"),
};
