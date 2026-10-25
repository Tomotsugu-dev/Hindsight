import { describe, expect, it } from "vitest";
import { pageLanguage } from "./i18n";

describe("pageLanguage", () => {
  it("picks the page language from ?lang=, then the browser, then en", () => {
    // 应用的按钮带的是界面语言，优先用它
    expect(pageLanguage("?lang=ja", ["zh-CN"])).toBe("ja");
    expect(pageLanguage("?lang=pt-BR", [])).toBe("pt-BR");
    // ?lang= 不认识：看浏览器
    expect(pageLanguage("?lang=fr", ["es-419", "en"])).toBe("es");
    // 浏览器的标签对到页面的语言；香港、繁体写法都算繁体中文
    expect(pageLanguage("", ["zh-HK"])).toBe("zh-TW");
    expect(pageLanguage("", ["zh-Hant-TW"])).toBe("zh-TW");
    expect(pageLanguage("", ["zh"])).toBe("zh-CN");
    expect(pageLanguage("", ["pt-PT"])).toBe("pt-BR");
    // 按浏览器给的顺序，跳过不认识的
    expect(pageLanguage("", ["fr-FR", "de", "ja-JP", "en-US"])).toBe("ja");
    // 都不认识
    expect(pageLanguage("", ["fr-FR"])).toBe("en");
    expect(pageLanguage("", [])).toBe("en");
  });
});
