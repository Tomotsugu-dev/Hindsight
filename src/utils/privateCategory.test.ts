// 用真的文案跑独立的 i18next 实例，断言写成用户看到的样子
import i18next, { type TFunction } from "i18next";
import { beforeAll, describe, expect, it } from "vitest";
import en from "../i18n/locales/en.json";
import es from "../i18n/locales/es.json";
import ja from "../i18n/locales/ja.json";
import ptBR from "../i18n/locales/pt-BR.json";
import zhCN from "../i18n/locales/zh-CN.json";
import zhTW from "../i18n/locales/zh-TW.json";
import { describeCount, purgeFailedReason } from "./privateCategory";

const LOCALES = { "zh-CN": zhCN, "zh-TW": zhTW, en, ja, es, "pt-BR": ptBR };

async function translator(lng: keyof typeof LOCALES): Promise<TFunction> {
  const i18n = i18next.createInstance();
  await i18n.init({ lng, resources: { [lng]: { translation: LOCALES[lng] } } });
  return i18n.t;
}

let zh: TFunction;
let enT: TFunction;
beforeAll(async () => {
  zh = await translator("zh-CN");
  enT = await translator("en");
});

const MB = 1024 * 1024;

describe("purgeFailedReason", () => {
  it("设置存好、删除失败的错误：取出删除失败的原因", () => {
    expect(purgeFailedReason("[PURGE_FAILED] disk full")).toBe("disk full");
    expect(purgeFailedReason(new Error("[PURGE_FAILED] disk full"))).toBe("disk full");
  });

  it("别的错误返回 null：设置没存上", () => {
    expect(purgeFailedReason("database is locked")).toBeNull();
  });
});

describe("describeCount", () => {
  it("数字按千分位写，截图后面带总大小", () => {
    const count = { activities: 1234, screenshots: 56, screenshotBytes: 210 * MB };
    expect(describeCount(zh, count)).toBe("已有的 1,234 条记录和 56 张截图（210.0 MB）");
  });

  it("没有截图时只说记录", () => {
    const count = { activities: 3, screenshots: 0, screenshotBytes: 0 };
    expect(describeCount(zh, count)).toBe("已有的 3 条记录");
  });

  it("还没统计出来，或者统计出错：说全部", () => {
    expect(describeCount(zh, null)).toBe("已有的全部记录和截图");
  });

  it("英文分单复数", () => {
    const one = { activities: 1, screenshots: 1, screenshotBytes: 2048 };
    expect(describeCount(enT, one)).toBe("the 1 record and 1 screenshot (2.0 KB) made so far");
    const many = { activities: 1234, screenshots: 56, screenshotBytes: 210 * MB };
    expect(describeCount(enT, many)).toBe(
      "the 1,234 records and 56 screenshots (210.0 MB) made so far",
    );
  });

  it("六种语言都有这几句，没有漏写的键", async () => {
    const count = { activities: 2, screenshots: 2, screenshotBytes: 2048 };
    for (const lng of Object.keys(LOCALES) as (keyof typeof LOCALES)[]) {
      const t = await translator(lng);
      expect(describeCount(t, count), lng).not.toContain("categories.private");
      expect(describeCount(t, null), lng).not.toContain("categories.private");
    }
  });
});
