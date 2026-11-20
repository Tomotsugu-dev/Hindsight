// Demo 专用 i18n（通过 web/vite.config.ts alias 替换 @app/i18n）。
// 主 Tauri 应用的 src/i18n/index.ts 保持静态打包不动；网页 demo 走公网，
// 把 6 份语言 JSON 改为按需动态 import，首屏只下载当前这 1 份语言包。
import i18n from "i18next";
import { initReactI18next } from "react-i18next";

export const LOCALE_STORAGE_KEY = "hindsight.locale";
export const FALLBACK_LOCALE = "en";

export type SupportedLocale = "zh-CN" | "zh-TW" | "en" | "ja" | "pt-BR" | "es";

const LOCALE_LOADERS: Record<
  SupportedLocale,
  () => Promise<{ default: Record<string, unknown> }>
> = {
  "zh-CN": () => import("@app/i18n/locales/zh-CN.json"),
  "zh-TW": () => import("@app/i18n/locales/zh-TW.json"),
  en: () => import("@app/i18n/locales/en.json"),
  ja: () => import("@app/i18n/locales/ja.json"),
  "pt-BR": () => import("@app/i18n/locales/pt-BR.json"),
  es: () => import("@app/i18n/locales/es.json"),
};

const inFlightLoads = new Map<SupportedLocale, Promise<void>>();

/** 把任意 BCP-47 locale 串映射到支持的六种之一 */
export function mapToSupported(loc: string | null | undefined): SupportedLocale {
  const l = (loc ?? "").toLowerCase();
  if (/^zh[-_]?(tw|hk|mo|hant)/.test(l)) return "zh-TW";
  if (l.startsWith("zh")) return "zh-CN";
  if (l.startsWith("ja")) return "ja";
  if (l.startsWith("pt")) return "pt-BR";
  if (l.startsWith("es")) return "es";
  return "en";
}

function readStoredLocale(): string | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage.getItem(LOCALE_STORAGE_KEY) : null;
  } catch {
    return null;
  }
}

function readQueryLocale(): string | null {
  try {
    return typeof location !== "undefined" ? new URLSearchParams(location.search).get("lang") : null;
  } catch {
    return null;
  }
}

function resolveInitialLocale(): SupportedLocale {
  const fromQuery = readQueryLocale();
  if (fromQuery) return mapToSupported(fromQuery);
  const stored = readStoredLocale();
  if (stored) return mapToSupported(stored);
  return "zh-CN";
}

void i18n.use(initReactI18next).init({
  resources: {},
  lng: resolveInitialLocale(),
  fallbackLng: FALLBACK_LOCALE,
  interpolation: {
    escapeValue: false,
  },
});

/** 按需加载并注册一种语言包（已加载或加载中会自动去重） */
export async function loadLocale(lng: SupportedLocale): Promise<void> {
  if (i18n.hasResourceBundle(lng, "translation")) return;

  const existing = inFlightLoads.get(lng);
  if (existing) return existing;

  const task = LOCALE_LOADERS[lng]()
    .then((mod) => {
      i18n.addResourceBundle(lng, "translation", mod.default, true, true);
    })
    .finally(() => {
      inFlightLoads.delete(lng);
    });

  inFlightLoads.set(lng, task);
  return task;
}

// 连续快速切语言时只让最后一次生效，避免慢请求回写覆盖新选择
let latestChangeSeq = 0;

/** 加载目标语言包并切换 i18n 当前语言，保证切换前翻译已就绪 */
export async function changeLocale(lng: string): Promise<void> {
  const target = mapToSupported(lng);
  const seq = ++latestChangeSeq;
  try {
    await loadLocale(target);
  } catch (e) {
    console.warn(`[demo/i18n] failed to load locale ${target}`, e);
    if (target !== FALLBACK_LOCALE) {
      await loadLocale(FALLBACK_LOCALE).catch(() => {});
    }
  }
  if (seq !== latestChangeSeq) return;
  await i18n.changeLanguage(target);
}

/** 渲染前加载当前页面所需的那 1 份语言包 */
export async function ensureInitialLocale(): Promise<void> {
  const target = resolveInitialLocale();
  try {
    await loadLocale(target);
  } catch (e) {
    console.warn(`[demo/i18n] failed to load initial locale ${target}`, e);
    if (target !== FALLBACK_LOCALE) {
      await loadLocale(FALLBACK_LOCALE).catch(() => {});
    }
  }
  if (i18n.language !== target) {
    await i18n.changeLanguage(target);
  }
}

export default i18n;
