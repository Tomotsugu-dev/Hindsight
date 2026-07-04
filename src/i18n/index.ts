// i18next 实例初始化
// - 已显式选择过语言：用 localStorage 持久化的值（key=hindsight.locale）
// - 首启未选择：渲染前 ensureInitialLocale() 按系统 locale 选语言（zh/ja→对应，其余→en）
// - 不引入 backend / detector 子包，资源直接静态 import
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { locale as osLocale } from "@tauri-apps/plugin-os";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import ptBR from "./locales/pt-BR.json";

export const LOCALE_STORAGE_KEY = "hindsight.locale";
/** 兜底语言：系统 locale 无法识别 / 非 zh,ja 时用 en（比中文通用） */
export const FALLBACK_LOCALE = "en";

type Supported = "zh-CN" | "zh-TW" | "en" | "ja" | "pt-BR";

/** 把任意 BCP-47 locale 串映射到支持的五种之一 */
function mapToSupported(loc: string | null | undefined): Supported {
  const l = (loc ?? "").toLowerCase();
  // 繁体圈（台湾 / 香港 / 澳门 / 显式 Hant 脚本）→ 繁体；其余中文 → 简体
  if (/^zh[-_]?(tw|hk|mo|hant)/.test(l)) return "zh-TW";
  if (l.startsWith("zh")) return "zh-CN";
  if (l.startsWith("ja")) return "ja";
  if (l.startsWith("pt")) return "pt-BR";
  return "en";
}

// 同步 init：有存储值就用；没有先用兜底，等 ensureInitialLocale 异步纠正
const stored = localStorage.getItem(LOCALE_STORAGE_KEY);

void i18n.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: zhCN },
    "zh-TW": { translation: zhTW },
    en: { translation: en },
    ja: { translation: ja },
    "pt-BR": { translation: ptBR },
  },
  lng: stored ?? FALLBACK_LOCALE,
  fallbackLng: FALLBACK_LOCALE,
  interpolation: {
    // React 自带 XSS 防护，无需 i18next 再做转义
    escapeValue: false,
  },
});

/**
 * 首启（未显式选择过语言）按系统 locale 设初始语言。必须在 React 首次渲染前 await，
 * 避免闪一下兜底语言。已有显式选择则原样尊重；自动识别**不写** localStorage——
 * 这样系统语言变了下次还能继续跟随。
 */
export async function ensureInitialLocale(): Promise<void> {
  if (localStorage.getItem(LOCALE_STORAGE_KEY)) return;
  let sys: string | null = null;
  try {
    sys = await osLocale();
  } catch {
    sys = null;
  }
  const target = mapToSupported(sys);
  if (i18n.language !== target) {
    await i18n.changeLanguage(target);
  }
}

export default i18n;
