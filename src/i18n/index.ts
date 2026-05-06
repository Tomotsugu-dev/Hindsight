// i18next 实例初始化
// - 使用 localStorage 持久化用户选择，key=hindsight.locale，默认 zh-CN
// - 不引入 backend / detector 子包，资源直接静态 import
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zhCN from "./locales/zh-CN.json";
import en from "./locales/en.json";

export const LOCALE_STORAGE_KEY = "hindsight.locale";
export const DEFAULT_LOCALE = "zh-CN";

const initialLocale =
  localStorage.getItem(LOCALE_STORAGE_KEY) ?? DEFAULT_LOCALE;

void i18n.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: zhCN },
    en: { translation: en },
  },
  lng: initialLocale,
  fallbackLng: DEFAULT_LOCALE,
  interpolation: {
    // React 自带 XSS 防护，无需 i18next 再做转义
    escapeValue: false,
  },
});

export default i18n;
