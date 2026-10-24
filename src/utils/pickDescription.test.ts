import { describe, it, expect } from "vitest";
import { pickDescription } from "./pickDescription";

describe("pickDescription", () => {
  const description = {
    "zh-CN": "离今天的目标还差多少",
    en: "How far you are from today's goal",
    ja: "今日の目標まであとどれくらい",
  };

  it("有当前界面语言就用它", () => {
    expect(pickDescription(description, "ja")).toBe("今日の目標まであとどれくらい");
  });

  it("没有当前语言就用英文", () => {
    expect(pickDescription(description, "pt-BR")).toBe("How far you are from today's goal");
  });

  it("语言代码要完全一致：繁体不拿简体顶替", () => {
    expect(pickDescription(description, "zh-TW")).toBe("How far you are from today's goal");
  });

  it("英文也没有，就用语言代码排序后的第一个", () => {
    expect(pickDescription({ ja: "日本語", "zh-CN": "简体", es: "Español" }, "pt-BR")).toBe(
      "Español",
    );
  });

  it("一种语言都没有就是空串", () => {
    expect(pickDescription({}, "en")).toBe("");
  });
});
