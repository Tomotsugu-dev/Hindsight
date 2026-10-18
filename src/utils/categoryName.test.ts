import { describe, it, expect, vi, beforeEach } from "vitest";
import zhCN from "../i18n/locales/zh-CN.json";
import zhTW from "../i18n/locales/zh-TW.json";
import en from "../i18n/locales/en.json";
import ja from "../i18n/locales/ja.json";
import ptBR from "../i18n/locales/pt-BR.json";
import es from "../i18n/locales/es.json";

type Names = Record<string, string>;
const translate = vi.fn<(categories: Names, superCategories: Names) => Promise<void>>();
vi.mock("../api/hindsight", () => ({
  api: {
    translateDefaultCategoryNames: (categories: Names, superCategories: Names) =>
      translate(categories, superCategories),
  },
}));
// 真的 i18n 一加载就读 localStorage，node 环境没有
vi.mock("../i18n", () => ({
  default: {
    t: (key: string) => (key === "categories.defaults" ? { code: "Coding" } : { work: "Work" }),
  },
}));

import { translateDefaultCategoryNames } from "./categoryName";

describe("translateDefaultCategoryNames", () => {
  beforeEach(() => {
    translate.mockReset();
  });

  it("把界面语言的默认分类、默认大类译名发给后端", async () => {
    translate.mockResolvedValue(undefined);
    await translateDefaultCategoryNames();
    expect(translate).toHaveBeenCalledWith({ code: "Coding" }, { work: "Work" });
  });

  it("后端失败不抛错，不挡启动", async () => {
    translate.mockRejectedValue(new Error("boom"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    await expect(translateDefaultCategoryNames()).resolves.toBeUndefined();
    warn.mockRestore();
  });

  // 缺一个译名，这个分类就一直是中文原名
  it("每种语言都有全部默认分类、默认大类的译名", () => {
    for (const locale of [zhCN, zhTW, en, ja, ptBR, es]) {
      expect(Object.keys(locale.categories.defaults).sort()).toEqual([
        "browse",
        "code",
        "design",
        "fun",
        "game",
        "hidden",
        "office",
        "other",
        "talk",
        "video",
        "workchat",
      ]);
      expect(Object.keys(locale.categories.super.defaults).sort()).toEqual([
        "browse",
        "play",
        "social",
        "work",
      ]);
    }
  });
});
