// 读写当前 locale 的 hook
// - 切换时同时调 i18n.changeLanguage 和写入 localStorage
import { useTranslation } from "react-i18next";
import { LOCALE_STORAGE_KEY } from "./index";

export type Locale = "zh-CN" | "en" | "ja" | "pt-BR";

// 语言切换列表（label 用各自语言的母语写法 endonym，不走 t()）。
// 设置页语言选择器 + 侧边栏 footer 切换器共用，加新语言只改这一处。
// 顺序也是侧边栏循环切换的顺序。
export const LOCALE_OPTIONS: { value: Locale; label: string }[] = [
  { value: "zh-CN", label: "简体中文" },
  { value: "en", label: "English" },
  { value: "ja", label: "日本語" },
  { value: "pt-BR", label: "Português" },
];

export function useLocale(): [Locale, (next: Locale) => void] {
  const { i18n } = useTranslation();
  const locale = (i18n.language as Locale) ?? "zh-CN";

  const setLocale = (next: Locale) => {
    void i18n.changeLanguage(next);
    localStorage.setItem(LOCALE_STORAGE_KEY, next);
  };

  return [locale, setLocale];
}
