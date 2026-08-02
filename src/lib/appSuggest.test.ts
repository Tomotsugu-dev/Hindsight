import { describe, expect, it } from "vitest";
import type { AppGroup } from "../api/hindsight";
import {
  buildAppSuggestions,
  filterAppSuggestions,
  SUGGEST_MAX,
} from "./appSuggest";

function group(
  displayName: string,
  categoryId: string | null,
  members: [string, number][],
): AppGroup {
  return {
    id: `g-${displayName}`,
    displayName,
    categoryId,
    members: members.map(([processName, recentSecs]) => ({
      processName,
      recentSecs,
      lastDeviceId: null,
    })),
  };
}

describe("buildAppSuggestions", () => {
  it("摊平所有组的成员", () => {
    const pool = buildAppSuggestions([
      group("Chrome", "web", [["Google Chrome", 100]]),
      group("VS Code", null, [
        ["Code", 50],
        ["Code Helper", 10],
      ]),
    ]);
    expect(pool.map((s) => s.process)).toEqual([
      "Code",
      "Code Helper",
      "Google Chrome",
    ]);
  });

  it("未分类优先,组内按近 7 天用时降序", () => {
    const pool = buildAppSuggestions([
      group("已分类的常用应用", "work", [["Busy", 9999]]),
      group("新应用", null, [
        ["Fresh Small", 5],
        ["Fresh Big", 500],
      ]),
    ]);
    // 用时最长的 Busy 仍排在未分类之后——收编新应用是最高频场景
    expect(pool.map((s) => s.process)).toEqual([
      "Fresh Big",
      "Fresh Small",
      "Busy",
    ]);
  });

  it("exclude 命中的进程不进池,且忽略大小写", () => {
    const pool = buildAppSuggestions(
      [group("组", null, [["Google Chrome", 10], ["Code", 5]])],
      ["  google chrome  "],
    );
    expect(pool.map((s) => s.process)).toEqual(["Code"]);
  });

  it("display 去掉可执行后缀(匹配用户在界面上看到的名字)", () => {
    const pool = buildAppSuggestions([group("组", null, [["chrome.exe", 1]])]);
    expect(pool[0]).toMatchObject({ process: "chrome.exe", display: "chrome" });
  });
});

describe("filterAppSuggestions", () => {
  const pool = buildAppSuggestions([
    group("浏览器", null, [["Google Chrome", 30]]),
    group("编辑器", null, [["Code", 20]]),
    group("微信", null, [["WeChat", 10]]),
  ]);

  it("空输入 = 不过滤", () => {
    expect(filterAppSuggestions(pool, "   ")).toHaveLength(3);
  });

  it("进程名子串命中,忽略大小写", () => {
    expect(filterAppSuggestions(pool, "CHROM").map((s) => s.process)).toEqual([
      "Google Chrome",
    ]);
  });

  it("组名也参与匹配(用户可能只记得中文名)", () => {
    expect(filterAppSuggestions(pool, "微信").map((s) => s.process)).toEqual([
      "WeChat",
    ]);
  });

  it("显示名参与匹配(进程名带后缀时)", () => {
    const withExe = buildAppSuggestions([
      group("组", null, [["notepad.exe", 1]]),
    ]);
    // 用户输 "notepad" 应该命中 notepad.exe
    expect(filterAppSuggestions(withExe, "notepad")).toHaveLength(1);
  });

  it("无命中返回空", () => {
    expect(filterAppSuggestions(pool, "zzz")).toEqual([]);
  });

  it("按 max 截断", () => {
    const many = buildAppSuggestions([
      group(
        "组",
        null,
        Array.from({ length: 20 }, (_, i) => [`App${i}`, i] as [string, number]),
      ),
    ]);
    expect(filterAppSuggestions(many, "")).toHaveLength(SUGGEST_MAX);
    expect(filterAppSuggestions(many, "", 3)).toHaveLength(3);
  });
});
