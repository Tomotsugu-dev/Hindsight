import { describe, expect, it } from "vitest";
import type { AppUsage, Category, SuperCategory } from "../api/hindsight";
import { UNGROUPED_ID, barMinutes, legendItems, rankByLevel, segmentsBySuper } from "./segments";

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

const cat = (id: string, superCategoryId: string | null): Category => ({
  id,
  name: id,
  color: "#000000",
  icon: "Folder",
  builtin: id === "hidden",
  apps: [],
  superCategoryId,
});

const sup = (id: string, sortOrder: number): SuperCategory => ({
  id,
  name: id,
  color: "#ffffff",
  icon: "Folder",
  sortOrder,
});

// 大类：工作在前、社交在后；编程和文档归工作，聊天归社交，摸鱼没归入，旧分类指向已删的大类
const SUPERS = [sup("work", 0), sup("social", 1)];
const CATS = [
  cat("code", "work"),
  cat("docs", "work"),
  cat("chat", "social"),
  cat("idle", null),
  cat("old", "deleted-super"),
  cat("hidden", null),
];

describe("segmentsBySuper", () => {
  it("同一大类的小类合成一段，秒数相加", () => {
    const parts = segmentsBySuper(
      [
        { categoryId: "code", secs: 1800 },
        { categoryId: "docs", secs: 600 },
        { categoryId: "chat", secs: 900 },
      ],
      CATS,
      SUPERS,
    );
    expect(parts).toEqual([
      { id: "work", secs: 2400 },
      { id: "social", secs: 900 },
    ]);
  });

  it("没归入大类的小类算进「未归入」", () => {
    expect(segmentsBySuper([{ categoryId: "idle", secs: 300 }], CATS, SUPERS)).toEqual([
      { id: UNGROUPED_ID, secs: 300 },
    ]);
  });

  it("所属大类已经删掉的小类也算进「未归入」", () => {
    expect(segmentsBySuper([{ categoryId: "old", secs: 300 }], CATS, SUPERS)).toEqual([
      { id: UNGROUPED_ID, secs: 300 },
    ]);
  });

  // 分类刚删、列表还没刷新：时长不能丢，否则柱子会比小类档矮一截
  it("找不到的分类算进「未归入」，柱子总秒数不变", () => {
    const segments = [
      { categoryId: "code", secs: 1200 },
      { categoryId: "gone", secs: 240 },
    ];
    const parts = segmentsBySuper(segments, CATS, SUPERS);
    expect(parts).toEqual([
      { id: "work", secs: 1200 },
      { id: UNGROUPED_ID, secs: 240 },
    ]);
    expect(parts.reduce((s, p) => s + p.secs, 0)).toBe(1440);
  });

  // 柱子从下往上叠：大类按分类页的顺序，「未归入」在最上面；和后端给的分段顺序无关
  it("按大类的顺序排，「未归入」排最后", () => {
    const parts = segmentsBySuper(
      [
        { categoryId: "idle", secs: 60 },
        { categoryId: "chat", secs: 60 },
        { categoryId: "code", secs: 60 },
      ],
      CATS,
      SUPERS,
    );
    expect(parts.map((p) => p.id)).toEqual(["work", "social", UNGROUPED_ID]);
  });

  it("没有时长的大类不出现", () => {
    expect(segmentsBySuper([{ categoryId: "chat", secs: 0 }], CATS, SUPERS)).toEqual([]);
  });
});

describe("legendItems", () => {
  it("小类档：列出隐藏以外的全部小类，和原来的图例一样", () => {
    expect(legendItems("cats", CATS, SUPERS, "未归入大类").map((i) => i.id)).toEqual([
      "code",
      "docs",
      "chat",
      "idle",
      "old",
    ]);
  });

  it("大类档：按顺序列出大类；有小类没归入时，最后加「未归入」", () => {
    const items = legendItems("supers", CATS, SUPERS, "未归入大类");
    expect(items.map((i) => i.id)).toEqual(["work", "social", UNGROUPED_ID]);
    expect(items[2].name).toBe("未归入大类");
  });

  it("大类档：小类都归入了大类，就没有「未归入」", () => {
    const grouped = CATS.filter((c) => c.superCategoryId === "work" || c.superCategoryId === "social");
    expect(legendItems("supers", grouped, SUPERS, "未归入大类").map((i) => i.id)).toEqual([
      "work",
      "social",
    ]);
  });
});

const app = (groupId: string, byCategory: [string, number][]): AppUsage => ({
  displayName: groupId,
  groupId,
  categoryId: byCategory[0][0],
  minutes: Math.round(byCategory.reduce((s, [, secs]) => s + secs, 0) / 60),
  iconProcess: `${groupId}.exe`,
  byCategory: byCategory.map(([categoryId, secs]) => ({ categoryId, secs })),
});

// 两个小时：9 点编程 + 聊天，10 点文档 + 摸鱼 + 一个刚删掉的分类
const SOURCES = [
  {
    segments: [
      { categoryId: "code", secs: 1800 },
      { categoryId: "chat", secs: 600 },
    ],
  },
  {
    segments: [
      { categoryId: "docs", secs: 1200 },
      { categoryId: "idle", secs: 300 },
      { categoryId: "gone", secs: 60 },
    ],
  },
];
// VS Code 在编程和文档里都有时间（同属工作）；Chrome 只在文档里；微信在聊天里
const APPS = [
  app("vscode", [
    ["code", 1800],
    ["docs", 300],
  ]),
  app("chrome", [["docs", 900]]),
  app("wechat", [["chat", 600]]),
];

describe("rankByLevel", () => {
  it("小类档：按小类加总各个时段的秒数，应用按在这个小类里的时长排", () => {
    const ranks = rankByLevel("cats", SOURCES, APPS, CATS, SUPERS);
    expect(ranks.get("code")).toEqual({ secs: 1800, iconProcesses: ["vscode.exe"] });
    expect(ranks.get("docs")).toEqual({ secs: 1200, iconProcesses: ["chrome.exe", "vscode.exe"] });
    expect(ranks.get("chat")?.secs).toBe(600);
  });

  it("大类档：按大类加总；没归入的和找不到的分类算进「未归入」", () => {
    const ranks = rankByLevel("supers", SOURCES, APPS, CATS, SUPERS);
    expect(ranks.get("work")?.secs).toBe(3000);
    expect(ranks.get("social")?.secs).toBe(600);
    expect(ranks.get(UNGROUPED_ID)?.secs).toBe(360);
    expect(ranks.has("code")).toBe(false);
  });

  // VS Code 在工作里是 1800 + 300 = 2100 秒，比 Chrome 的 900 秒多，排第一，而且只出现一次
  it("大类档：同一个应用在一个大类的几个小类里都有时间，只列一次，按合计时长排", () => {
    const ranks = rankByLevel("supers", SOURCES, APPS, CATS, SUPERS);
    expect(ranks.get("work")?.iconProcesses).toEqual(["vscode.exe", "chrome.exe"]);
  });
});
