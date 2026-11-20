import { describe, expect, it } from "vitest";
import type { ModuleProfiles } from "../api/hindsight";
import { nextProfileName, profileProblems, resolveSelected, sortProfiles } from "./aiProfiles";

const work = { name: "工作", categories: ["code"] };
const study = { name: "学习", categories: ["office"] };

describe("resolveSelected", () => {
  const withAuto: ModuleProfiles = { profiles: { p_work: work }, autoProfile: "p_work" };

  it("没选过时，日报跟着自动生成用的那套", () => {
    expect(resolveSelected("daily", withAuto, undefined)).toBe("p_work");
  });

  it("聊天没有自动生成，没选过就是默认分类", () => {
    expect(resolveSelected("chat", withAuto, undefined)).toBeNull();
  });

  it("选了默认分类就是默认分类，不被自动生成的那套顶掉", () => {
    expect(resolveSelected("daily", withAuto, null)).toBeNull();
  });

  it("选中的那套被删了，回到默认", () => {
    expect(resolveSelected("daily", withAuto, "p_gone")).toBe("p_work");
  });

  it("自动生成用的那套被删了，回到默认分类", () => {
    const gone: ModuleProfiles = { profiles: {}, autoProfile: "p_work" };
    expect(resolveSelected("weekly", gone, undefined)).toBeNull();
  });

  it("原型上的名字不算配置", () => {
    expect(resolveSelected("daily", { profiles: {}, autoProfile: null }, "toString")).toBeNull();
  });
});

describe("profileProblems", () => {
  it("名字、分类都齐就没有问题", () => {
    expect(profileProblems({ a: work, b: study })).toEqual({});
  });

  it("名字全是空格算没名字", () => {
    expect(profileProblems({ a: { name: "  ", categories: ["code"] }, b: study })).toEqual({
      a: "aiProfiles.errors.noName",
    });
  });

  it("去掉首尾空格后重名，重名的几套都标出来", () => {
    expect(
      profileProblems({ a: work, b: { name: " 工作 ", categories: ["x"] }, c: study }),
    ).toEqual({
      a: "aiProfiles.errors.sameName",
      b: "aiProfiles.errors.sameName",
    });
  });

  it("一个分类都没选", () => {
    expect(profileProblems({ a: { name: "空", categories: [] } })).toEqual({
      a: "aiProfiles.errors.noCategory",
    });
  });

  it("一套有好几个问题时只报名字的", () => {
    expect(profileProblems({ a: { name: "", categories: [] } })).toEqual({
      a: "aiProfiles.errors.noName",
    });
  });
});

describe("nextProfileName", () => {
  it("跳过已经用掉的编号", () => {
    const used = { a: { name: "配置 1", categories: [] }, b: { name: "配置 2", categories: [] } };
    expect(nextProfileName(used, (n) => `配置 ${n}`)).toBe("配置 3");
  });
});

describe("sortProfiles", () => {
  it("按名字排", () => {
    const names = sortProfiles({
      z: { name: "B", categories: [] },
      a: { name: "A", categories: [] },
    });
    expect(names.map(([id]) => id)).toEqual(["a", "z"]);
  });
});
