import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// 本文件只测 completedDaysOf / catMinutesFromSegments 两个纯函数;
// 但模块顶层经 state/categories 间接 import @tauri-apps/api/core,
// 且自身 import react-i18next——node 环境把两者都桩掉,避免加载副作用。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.resolve(null),
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

import {
  catMinutesFromSegments,
  completedDaysOf,
} from "./useSuperCategoryBreakdown";

describe("completedDaysOf(日均分母口径:只算已完成自然天)", () => {
  beforeEach(() => {
    // 固定"现在",让今天零点边界可精确断言(避免真跑在午夜翻转瞬间抖动)
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 6, 26, 15, 30, 0)); // 2026-07-26 15:30 本地
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("严格早于今天零点的保留;今天(进行中)与未来剔除", () => {
    const days = [
      { date: new Date(2026, 6, 24), label: "前天" },
      { date: new Date(2026, 6, 25, 23, 59, 59), label: "昨天最后一秒" },
      { date: new Date(2026, 6, 26), label: "今天零点" },
      { date: new Date(2026, 6, 26, 9, 0), label: "今天上午" },
      { date: new Date(2026, 6, 27), label: "明天" },
    ];
    // 今天只过了半天,计入分母会让"日均"失真——半天的数据配一整天的分母;
    // 今天零点这个精确边界值也必须剔除(d.date < 今天零点 是严格小于)
    expect(completedDaysOf(days).map((d) => d.label)).toEqual([
      "前天",
      "昨天最后一秒",
    ]);
  });

  it("过去周期(整周都早于今天)天然全量保留", () => {
    const week = Array.from({ length: 7 }, (_, i) => ({
      date: new Date(2026, 5, 1 + i),
    }));
    expect(completedDaysOf(week)).toHaveLength(7);
  });

  it("空输入 → 空输出", () => {
    expect(completedDaysOf([])).toEqual([]);
  });
});

describe("catMinutesFromSegments(先累秒后取整口径)", () => {
  it("跨源、跨桶按 categoryId 累秒,最后一步才换算分钟", () => {
    const out = catMinutesFromSegments([
      {
        segments: [
          { categoryId: "work", secs: 100 },
          { categoryId: "play", secs: 240 },
        ],
      },
      { segments: [{ categoryId: "work", secs: 100 }] },
      { segments: [{ categoryId: "work", secs: 100 }] },
    ]);
    const byId = new Map(out.map((c) => [c.id, c.minutes]));
    // work 总计 300s = 恰好 5 分钟;play 240s = 4 分钟
    expect(byId.get("work")).toBe(5);
    expect(byId.get("play")).toBe(4);
  });

  it("偏差样例:逐桶取整会把 3×100s 算成 6 分钟,先累秒后取整是 5 分钟", () => {
    // 逐桶口径:round(100/60)=2,三桶 2+2+2=6 —— 每桶各多"送"约 0.33 分钟,
    // 碎片使用越多系统性偏得越多;正确口径是 round(300/60)=5,
    // 与 top-apps 的"先加总后取整"保持一致,两边数字才对得上。
    const buckets = Array.from({ length: 3 }, () => ({
      segments: [{ categoryId: "work", secs: 100 }],
    }));
    const perBucketSum = buckets
      .map((b) => Math.round(b.segments[0].secs / 60))
      .reduce((a, b) => a + b, 0);
    expect(perBucketSum).toBe(6); // 锚定"错误口径确实会偏"这一前提
    expect(catMinutesFromSegments(buckets)).toEqual([
      { id: "work", minutes: 5 },
    ]);
  });

  it("四舍五入在总和上做:29s → 0 分钟,30s → 1 分钟", () => {
    expect(
      catMinutesFromSegments([{ segments: [{ categoryId: "a", secs: 29 }] }]),
    ).toEqual([{ id: "a", minutes: 0 }]);
    expect(
      catMinutesFromSegments([{ segments: [{ categoryId: "a", secs: 30 }] }]),
    ).toEqual([{ id: "a", minutes: 1 }]);
  });

  it("0 秒分类也返回条目(minutes=0),过滤是下游 breakdown 的职责", () => {
    expect(
      catMinutesFromSegments([{ segments: [{ categoryId: "idle", secs: 0 }] }]),
    ).toEqual([{ id: "idle", minutes: 0 }]);
  });

  it("空源 / 空 segments → 空数组", () => {
    expect(catMinutesFromSegments([])).toEqual([]);
    expect(catMinutesFromSegments([{ segments: [] }])).toEqual([]);
  });
});
