import { describe, expect, it } from "vitest";
import { creationScrollTarget } from "./creationFeedback";

describe("creationScrollTarget", () => {
  it("没有新建时不滚", () => {
    expect(creationScrollTarget(null, null, null)).toBeNull();
  });

  it("新建分类 → 滚到该分类行", () => {
    expect(creationScrollTarget("cat-1", null, null)).toEqual({
      key: "cat-1",
      kind: "cat",
    });
  });

  it("新建大类 → 滚到该大类卡", () => {
    expect(creationScrollTarget(null, "sup-1", null)).toEqual({
      key: "sup-1",
      kind: "super",
    });
  });

  it("同一个新建只滚一次", () => {
    // 表格每次 drag / hover / refresh 都重渲染;不记住已滚过的 id,
    // 用户拖分类时会被反复弹回新建那一行
    expect(creationScrollTarget("cat-1", null, "cat-1")).toBeNull();
    expect(creationScrollTarget(null, "sup-1", "sup-1")).toBeNull();
  });

  it("换了个新建目标就重新滚", () => {
    expect(creationScrollTarget("cat-2", null, "cat-1")).toEqual({
      key: "cat-2",
      kind: "cat",
    });
  });

  it("两个 id 都在时取分类(调用方每次新建都清另一个,分类即最新)", () => {
    expect(creationScrollTarget("cat-1", "sup-9", null)).toEqual({
      key: "cat-1",
      kind: "cat",
    });
  });
});
