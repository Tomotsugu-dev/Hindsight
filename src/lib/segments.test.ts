import { describe, expect, it } from "vitest";
import { barMinutes } from "./segments";

describe("barMinutes", () => {
  // 三个分类各 40 秒：先加秒数再取整是 2 分钟；各段先取整再相加会变成 3 分钟，
  // 和页面顶部按秒算的总时长对不上
  it("把秒数加起来再取整，不把各段取整后的分钟相加", () => {
    const segments = ["work", "fun", "other"].map((categoryId) => ({ categoryId, secs: 40 }));
    expect(barMinutes(segments)).toBe(2);
  });

  it("没有分段时是 0", () => {
    expect(barMinutes([])).toBe(0);
  });
});
