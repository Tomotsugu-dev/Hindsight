import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n, {
  changeLocale,
  ensureInitialLocale,
  loadLocale,
  LOCALE_STORAGE_KEY,
  mapToSupported,
} from "./i18n-demo";

describe("demo i18n 按需加载", () => {
  const storage = new Map<string, string>();

  beforeEach(() => {
    storage.clear();
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => storage.get(k) ?? null,
      setItem: (k: string, v: string) => {
        storage.set(k, v);
      },
      removeItem: (k: string) => {
        storage.delete(k);
      },
    });
    vi.stubGlobal("location", { search: "" });
    for (const lng of ["zh-CN", "zh-TW", "en", "ja", "pt-BR", "es"]) {
      if (i18n.hasResourceBundle(lng, "translation")) {
        i18n.removeResourceBundle(lng, "translation");
      }
    }
  });

  it("mapToSupported 正确映射常见 BCP-47 地区码", () => {
    expect(mapToSupported("zh-CN")).toBe("zh-CN");
    expect(mapToSupported("zh-Hant-TW")).toBe("zh-TW");
    expect(mapToSupported("ja-JP")).toBe("ja");
    expect(mapToSupported("pt-BR")).toBe("pt-BR");
    expect(mapToSupported("es-MX")).toBe("es");
    expect(mapToSupported("fr-FR")).toBe("en");
  });

  it("ensureInitialLocale 只按需加载当前语言包", async () => {
    vi.stubGlobal("location", { search: "?lang=ja" });

    await ensureInitialLocale();

    expect(i18n.language).toBe("ja");
    expect(i18n.hasResourceBundle("ja", "translation")).toBe(true);
    expect(i18n.hasResourceBundle("zh-CN", "translation")).toBe(false);
    expect(i18n.hasResourceBundle("en", "translation")).toBe(false);
  });

  it("changeLocale 在切换前加载目标语言包并更新语言", async () => {
    await changeLocale("es");

    expect(i18n.language).toBe("es");
    expect(i18n.hasResourceBundle("es", "translation")).toBe(true);
    expect(i18n.t("nav.items.today")).not.toBe("nav.items.today");
  });

  it("loadLocale 并发调用去重并成功注册资源", async () => {
    await Promise.all([loadLocale("pt-BR"), loadLocale("pt-BR")]);
    expect(i18n.hasResourceBundle("pt-BR", "translation")).toBe(true);
  });

  it("优先从 localStorage 读取已选语言", async () => {
    storage.set(LOCALE_STORAGE_KEY, "zh-TW");
    await ensureInitialLocale();
    expect(i18n.language).toBe("zh-TW");
    expect(i18n.hasResourceBundle("zh-TW", "translation")).toBe(true);
  });
});
