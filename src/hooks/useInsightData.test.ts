// 取代 usePeriodInsights.test.ts：下钻、选中小类、悬停应用时，卡片算哪些时间、主力卡片显示什么
import { describe, expect, it } from "vitest";
import type { AppUsage, Category } from "../api/hindsight";
import { UNGROUPED_ID } from "../lib/segments";
import { dominantShare, focusCategories } from "./useInsightData";
import type { BreakdownSlice } from "./useSuperCategoryBreakdown";

const cat = (id: string, superCategoryId: string | null, builtin = false): Category => ({
  id,
  name: id,
  color: `#${id.length}00000`,
  icon: "Folder",
  builtin,
  apps: [],
  superCategoryId,
});

const categories = [
  cat("code", "work"),
  cat("docs", "work"),
  cat("video", "fun"),
  cat("loose", null),
  cat("hidden", null, true),
];
const superIds = new Set(["work", "fun"]);

describe("focusCategories", () => {
  it("counts every category of a drilled super category, also those with no time", () => {
    expect(focusCategories({ kind: "super", superId: "work" }, categories, superIds)).toEqual(
      new Set(["code", "docs"]),
    );
  });

  it("counts the categories in no super category for Ungrouped, without built-in ones", () => {
    expect(focusCategories({ kind: "super", superId: UNGROUPED_ID }, categories, superIds)).toEqual(
      new Set(["loose"]),
    );
  });

  it("counts only the picked category, and everything when nothing is picked", () => {
    expect(focusCategories({ kind: "cat", catId: "docs" }, categories, superIds)).toEqual(
      new Set(["docs"]),
    );
    expect(focusCategories({ kind: "all" }, categories, superIds)).toBeNull();
  });
});

describe("dominantShare", () => {
  // 工作 90 分（编程 60、文档 30），娱乐 30 分，另有 30 分不进圆环的内置分类
  const slices: BreakdownSlice[] = [
    {
      id: "work",
      name: "Work",
      color: "#111111",
      icon: "Folder",
      minutes: 90,
      cats: [
        { id: "code", name: "Code", color: "#222222", minutes: 60 },
        { id: "docs", name: "Docs", color: "#333333", minutes: 30 },
      ],
    },
    { id: "fun", name: "Fun", color: "#444444", icon: "Folder", minutes: 30, cats: [] },
  ];
  const rows = [
    {
      date: "2026-11-05",
      segments: [
        { categoryId: "code", secs: 3600 },
        { categoryId: "docs", secs: 1800 },
        { categoryId: "video", secs: 1800 },
        { categoryId: "hidden", secs: 1800 },
      ],
    },
  ];

  it("shows the biggest super category's share of all the time", () => {
    // 90 / 150，内置分类的时间也算进总数，和其它卡片一样
    expect(dominantShare({ kind: "all" }, slices, rows, undefined, undefined)).toEqual({
      kind: "share",
      name: "Work",
      color: "#111111",
      pct: 60,
    });
  });

  it("shows the biggest category inside a drilled super category", () => {
    expect(
      dominantShare({ kind: "super", superId: "work" }, slices, rows, undefined, undefined),
    ).toMatchObject({ name: "Code", pct: 67 });
  });

  it("shows nothing for a picked category", () => {
    expect(dominantShare({ kind: "cat", catId: "code" }, slices, rows, undefined, undefined)).toBe(
      null,
    );
  });

  it("shows a hovered app's share of its own category", () => {
    // 浏览器 40 分里 20 分算编程（网站规则分走一部分），编程一共 60 分
    const apps = [
      {
        groupId: "browser",
        byCategory: [
          { categoryId: "code", secs: 1200 },
          { categoryId: "video", secs: 1200 },
        ],
      } as AppUsage,
    ];
    expect(
      dominantShare(
        { kind: "app", groupId: "browser", categoryId: "code" },
        slices,
        rows,
        apps,
        categories[0],
      ),
    ).toMatchObject({ name: "code", pct: 33 });
  });
});
