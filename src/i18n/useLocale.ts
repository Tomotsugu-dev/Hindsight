// 读写当前 locale 的 hook
// - 切换时同时调 i18n.changeLanguage 和写入 localStorage
import { useTranslation } from "react-i18next";
import { LOCALE_STORAGE_KEY } from "./index";

export type Locale = "zh-CN" | "en";

export function useLocale(): [Locale, (next: Locale) => void] {
  const { i18n } = useTranslation();
  const locale = (i18n.language as Locale) ?? "zh-CN";

  const setLocale = (next: Locale) => {
    void i18n.changeLanguage(next);
    localStorage.setItem(LOCALE_STORAGE_KEY, next);
  };

  return [locale, setLocale];
}
