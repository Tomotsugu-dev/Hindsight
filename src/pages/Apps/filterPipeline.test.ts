import { describe, it, expect } from "vitest";
import type { AppGroup } from "../../api/hindsight";
import type { AppsFilter } from "./useAppsFilter";
import { applyFilter, totalRecentSecs } from "./filterPipeline";

// —— fixtures ——
function member(processName: string, recentSecs: number): AppGroup["members"][number] {
  return { processName, recentSecs, lastDeviceId: null };
}

function group(
  id: string,
  displayName: string,
  categoryId: string | null,
  members: AppGroup["members"],
): AppGroup {
  return { id, displayName, categoryId, members };
}

const GROUPS: AppGroup[] = [
  group("g1", "VSCode", "code", [member("code.exe", 300), member("helper.exe", 100)]),
  group("g2", "Chrome", "browse", [member("chrome.exe", 600)]),
  group("g3", "Notepad", null, [member("notepad.exe", 50)]),
  group("g4", "Slack", "talk", [member("slack.exe", 200)]),
];

const BASE: AppsFilter = {
  search: "",
  selectedCategoryIds: [],
  unassignedOnly: false,
  sortBy: "default",
};

describe("totalRecentSecs", () => {
  it("把组内所有成员的 recentSecs 求和", () => {
    expect(totalRecentSecs(GROUPS[0])).toBe(400);
    expect(totalRecentSecs(GROUPS[1])).toBe(600);
  });

  it("空组返回 0", () => {
    expect(totalRecentSecs(group("e", "Empty", null, []))).toBe(0);
  });
});

describe("applyFilter", () => {
  it("default 透传：不过滤不排序，保持入参顺序", () => {
    const out = applyFilter(GROUPS, BASE);
    expect(out.map((g) => g.id)).toEqual(["g1", "g2", "g3", "g4"]);
  });

  it("不 mutate 入参，返回新数组", () => {
    const before = GROUPS.map((g) => g.id);
    const out = applyFilter(GROUPS, { ...BASE, sortBy: "duration_desc" });
    expect(out).not.toBe(GROUPS);
    expect(GROUPS.map((g) => g.id)).toEqual(before); // 原数组顺序不变
  });

  it("unassignedOnly 排他：只留 categoryId === null 的组", () => {
    const out = applyFilter(GROUPS, { ...BASE, unassignedOnly: true });
    expect(out.map((g) => g.id)).toEqual(["g3"]);
  });

  it("unassignedOnly 优先于 selectedCategoryIds", () => {
    const out = applyFilter(GROUPS, {
      ...BASE,
      unassignedOnly: true,
      selectedCategoryIds: ["code"], // 应被 unassignedOnly 短路忽略
    });
    expect(out.map((g) => g.id)).toEqual(["g3"]);
  });

  it("selectedCategoryIds 过滤：只留命中分类的组", () => {
    const out = applyFilter(GROUPS, {
      ...BASE,
      selectedCategoryIds: ["code", "talk"],
    });
    expect(out.map((g) => g.id).sort()).toEqual(["g1", "g4"]);
  });

  it("空 selectedCategoryIds 视为不限分类（pass-through）", () => {
    const out = applyFilter(GROUPS, { ...BASE, selectedCategoryIds: [] });
    expect(out).toHaveLength(GROUPS.length);
  });

  it("search 匹配 displayName，不区分大小写", () => {
    const out = applyFilter(GROUPS, { ...BASE, search: "chrome" });
    expect(out.map((g) => g.id)).toEqual(["g2"]);
  });

  it("search 也匹配任一 member.processName", () => {
    const out = applyFilter(GROUPS, { ...BASE, search: "helper" });
    expect(out.map((g) => g.id)).toEqual(["g1"]);
  });

  it("search 前后空白被 trim", () => {
    const out = applyFilter(GROUPS, { ...BASE, search: "  slack  " });
    expect(out.map((g) => g.id)).toEqual(["g4"]);
  });

  it("duration_desc 按总时长降序", () => {
    const out = applyFilter(GROUPS, { ...BASE, sortBy: "duration_desc" });
    expect(out.map((g) => g.id)).toEqual(["g2", "g1", "g4", "g3"]);
  });

  it("duration_asc 按总时长升序", () => {
    const out = applyFilter(GROUPS, { ...BASE, sortBy: "duration_asc" });
    expect(out.map((g) => g.id)).toEqual(["g3", "g4", "g1", "g2"]);
  });

  it("name_asc / name_desc 按 displayName 排序", () => {
    const asc = applyFilter(GROUPS, { ...BASE, sortBy: "name_asc" });
    expect(asc.map((g) => g.displayName)).toEqual([
      "Chrome",
      "Notepad",
      "Slack",
      "VSCode",
    ]);
    const desc = applyFilter(GROUPS, { ...BASE, sortBy: "name_desc" });
    expect(desc.map((g) => g.displayName)).toEqual([
      "VSCode",
      "Slack",
      "Notepad",
      "Chrome",
    ]);
  });

  it("过滤 + 排序组合：分类过滤后再按时长降序", () => {
    const out = applyFilter(GROUPS, {
      ...BASE,
      selectedCategoryIds: ["code", "browse"],
      sortBy: "duration_desc",
    });
    expect(out.map((g) => g.id)).toEqual(["g2", "g1"]);
  });
});
