import { describe, expect, it } from "vitest";
import type { AiVisibility } from "../api/hindsight";
import {
  clearAuto,
  nextProfileName,
  profileProblems,
  resolveSelected,
  sortProfiles,
} from "./aiProfiles";

const work = { name: "工作", categories: ["code"] };
const study = { name: "学习", categories: ["office"] };

describe("resolveSelected", () => {
  // 一个池子，日报、周报自动生成各选一套
  const pool: AiVisibility = {
    profiles: { p_work: work, p_study: study },
    auto: { daily: "p_work", weekly: "p_study" },
  };

  it("没选过时，日报、周报各跟着自己自动生成用的那套", () => {
    expect(resolveSelected("daily", pool, undefined)).toBe("p_work");
    expect(resolveSelected("weekly", pool, undefined)).toBe("p_study");
  });

  it("聊天没有自动生成，没选过就是默认分类", () => {
    expect(resolveSelected("chat", pool, undefined)).toBeNull();
  });

  it("聊天能选池子里的任何一套", () => {
    expect(resolveSelected("chat", pool, "p_study")).toBe("p_study");
  });

  it("选了默认分类就是默认分类，不被自动生成的那套顶掉", () => {
    expect(resolveSelected("daily", pool, null)).toBeNull();
  });

  it("选中的那套被删了，回到默认", () => {
    expect(resolveSelected("daily", pool, "p_gone")).toBe("p_work");
  });

  it("自动生成用的那套被删了，回到默认分类", () => {
    const gone: AiVisibility = { profiles: {}, auto: { daily: null, weekly: "p_work" } };
    expect(resolveSelected("weekly", gone, undefined)).toBeNull();
  });

  it("原型上的名字不算配置", () => {
    const empty: AiVisibility = { profiles: {}, auto: { daily: null, weekly: null } };
    expect(resolveSelected("daily", empty, "toString")).toBeNull();
  });
});

describe("clearAuto", () => {
  it("删掉的那套正被自动生成用着，用它的报告回到默认分类，别的不动", () => {
    expect(clearAuto({ daily: "p_work", weekly: "p_study" }, "p_work")).toEqual({
      daily: null,
      weekly: "p_study",
    });
    expect(clearAuto({ daily: "p_work", weekly: "p_work" }, "p_work")).toEqual({
      daily: null,
      weekly: null,
    });
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
