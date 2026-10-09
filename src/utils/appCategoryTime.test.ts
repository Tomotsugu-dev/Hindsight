import { describe, it, expect } from "vitest";
import { appsInCategories, secsIn } from "./appCategoryTime";

// 语义第 9 条的例子：Chrome 35 分 =「浏览」25 分 + bilibili 归「影音」10 分；Safari 15 分全在「浏览」。
const chrome = {
  name: "Chrome",
  minutes: 35,
  byCategory: [
    { categoryId: "browse", secs: 1500 },
    { categoryId: "video", secs: 600 },
  ],
};
const safari = { name: "Safari", minutes: 15, byCategory: [{ categoryId: "browse", secs: 900 }] };

const shown = (apps: { name: string; minutes: number }[]) => apps.map((a) => [a.name, a.minutes]);

describe("appsInCategories", () => {
  it("选中「影音」：Chrome 只显示 bilibili 那 10 分，Safari 不出现", () => {
    expect(shown(appsInCategories([chrome, safari], new Set(["video"])))).toEqual([["Chrome", 10]]);
  });

  it("选中「浏览」：两个都在，按在这个分类里的时长排，加起来等于「浏览」的 40 分", () => {
    const apps = appsInCategories([safari, chrome], new Set(["browse"]));
    expect(shown(apps)).toEqual([
      ["Chrome", 25],
      ["Safari", 15],
    ]);
    expect(apps.reduce((sum, a) => sum + a.minutes, 0)).toBe(40);
  });

  it("钉住包含两个分类的大类：Chrome 两部分加起来", () => {
    expect(shown(appsInCategories([chrome, safari], new Set(["browse", "video"])))).toEqual([
      ["Chrome", 35],
      ["Safari", 15],
    ]);
  });

  it("不改原来的对象", () => {
    appsInCategories([chrome], new Set(["video"]));
    expect(chrome.minutes).toBe(35);
  });
});

describe("secsIn", () => {
  it("没有 byCategory 按 0 算", () => {
    expect(secsIn(undefined, new Set(["browse"]))).toBe(0);
  });
});
