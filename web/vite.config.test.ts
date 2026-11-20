import { describe, expect, it } from "vitest";
import config from "./vite.config";

describe("web/vite.config alias 顺序", () => {
  it("i18n 与 useLocale 的正则 alias 必须排在 @app 之前，避免 @app/i18n 绕过按需加载", () => {
    const aliases = config.resolve?.alias;
    expect(Array.isArray(aliases)).toBe(true);
    if (!Array.isArray(aliases)) return;

    const appIdx = aliases.findIndex((a) => a.find === "@app");
    const i18nIdx = aliases.findIndex(
      (a) => a.find instanceof RegExp && a.find.test("@app/i18n") && !a.find.test("@app/i18n/locales/en.json"),
    );
    const useLocaleIdx = aliases.findIndex(
      (a) => a.find instanceof RegExp && a.find.test("@app/i18n/useLocale"),
    );

    expect(appIdx).toBeGreaterThan(-1);
    expect(i18nIdx).toBeGreaterThan(-1);
    expect(useLocaleIdx).toBeGreaterThan(-1);
    expect(i18nIdx).toBeLessThan(appIdx);
    expect(useLocaleIdx).toBeLessThan(appIdx);
  });
});
