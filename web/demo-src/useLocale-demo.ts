// Demo 专用 useLocale（通过 web/vite.config.ts alias 替换 @app/i18n/useLocale）。
// 切换语言时先按需动态加载目标语言 JSON，再切换 i18n 语言，避免闪出原始 key。
import { useTranslation } from "react-i18next";
import { changeLocale, LOCALE_STORAGE_KEY } from "./i18n-demo";

export type Locale = "zh-CN" | "zh-TW" | "en" | "ja" | "pt-BR" | "es";

export const LOCALE_OPTIONS: { value: Locale; label: string }[] = [
  { value: "zh-CN", label: "简体中文" },
  { value: "zh-TW", label: "繁體中文" },
  { value: "en", label: "English" },
  { value: "ja", label: "日本語" },
  { value: "pt-BR", label: "Português" },
  { value: "es", label: "Español" },
];

export function useLocale(): [Locale, (next: Locale) => void] {
  const { i18n } = useTranslation();
  const locale = (i18n.language as Locale) ?? "zh-CN";

  const setLocale = (next: Locale) => {
    try {
      localStorage.setItem(LOCALE_STORAGE_KEY, next);
    } catch {
      // 隐私模式等环境下 localStorage 可能不可写
    }
    void changeLocale(next);
  };

  return [locale, setLocale];
}
