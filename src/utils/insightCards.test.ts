import { describe, expect, it } from "vitest";
import type { HourSlot } from "../api/hindsight";
import {
  appSeries,
  averageEdges,
  averageMinutes,
  cardValue,
  categorySeries,
  dayEdges,
  deviceShares,
  peakDay,
  stretchSecs,
  widestEdges,
  type CardInputs,
} from "./insightCards";
import { insightWindows } from "./insightWindows";

/** 一天 24 个小时格；`busy` 里列出的小时各有 `secs` 秒，分在 work 和 fun 两个分类 */
function hoursOf(busy: number[], secs = 600): HourSlot[] {
  return Array.from({ length: 24 }, (_, hour) => ({
    hour,
    segments: busy.includes(hour)
      ? [
          { categoryId: "work", secs: secs / 2 },
          { categoryId: "fun", secs: secs / 2 },
        ]
      : [],
  }));
}

const day = (date: string, work: number, fun = 0) => ({
  date,
  segments: [
    { categoryId: "work", secs: work },
    { categoryId: "fun", secs: fun },
  ],
});

describe("stretchSecs", () => {
  const series = categorySeries(
    [day("2026-10-26", 3600), day("2026-10-27", 7200)],
    [{ date: "2026-10-28", hours: hoursOf([9, 15], 1200) }],
    null,
  );

  it("adds whole days, then the hours of the cut day before the cut", () => {
    // 10-28 截到 15:30：9 点那格全算（1200），15 点那格算一半（600）
    const secs = stretchSecs(series, {
      days: { from: "2026-10-26", to: "2026-10-27" },
      part: { date: "2026-10-28", hour: 15, fraction: 0.5 },
    });
    expect(secs).toBe(3600 + 7200 + 1200 + 600);
  });

  it("is null while a day it needs is not loaded", () => {
    expect(
      stretchSecs(series, { days: { from: "2026-10-25", to: "2026-10-26" }, part: null }),
    ).toBe(null);
    expect(
      stretchSecs(series, { days: null, part: { date: "2026-10-29", hour: 3, fraction: 0 } }),
    ).toBeNull();
  });
});

describe("categorySeries", () => {
  it("counts only the picked categories", () => {
    const series = categorySeries([day("2026-10-26", 3600, 1800)], [], new Set(["fun"]));
    expect(series.days.get("2026-10-26")).toBe(1800);
  });
});

describe("averageMinutes and peakDay", () => {
  // 三天：60 分、0 分、120 分
  const series = categorySeries(
    [day("2026-10-26", 3600), day("2026-10-27", 0), day("2026-10-28", 7200)],
    [],
    null,
  );
  const range = { from: "2026-10-26", to: "2026-10-28" };

  it("divides by every day of the range, days without records included", () => {
    expect(averageMinutes(series, range)).toBe(60);
  });

  it("finds the day with the most time", () => {
    expect(peakDay(series, range)).toEqual({ date: "2026-10-28", secs: 7200 });
  });
});

describe("start and end times", () => {
  const series = categorySeries(
    [],
    [
      { date: "2026-11-02", hours: hoursOf([9, 10, 18]) },
      { date: "2026-11-03", hours: hoursOf([9, 19]) },
      { date: "2026-11-04", hours: [] },
      { date: "2026-11-05", hours: hoursOf([10, 18]) },
    ],
    null,
  );
  const dates = ["2026-11-02", "2026-11-03", "2026-11-04", "2026-11-05"];

  it("ends a day at the end of its last hour with records", () => {
    expect(dayEdges(series.hours.get("2026-11-02")!)).toEqual({ start: 9, end: 19 });
    expect(dayEdges(series.hours.get("2026-11-04")!)).toBeNull();
  });

  it("takes the earliest start and the latest end, skipping days without records", () => {
    expect(widestEdges(series, dates)).toEqual({ start: 9, end: 20 });
  });

  it("averages starts and ends over days with records, rounded to the hour", () => {
    // 开始 9、9、10 → 9.33 → 9；结束 19、20、19 → 19.33 → 19
    expect(averageEdges(series, dates)).toEqual({ start: 9, end: 19 });
  });

  it("is null until every day's hours are loaded", () => {
    expect(widestEdges(series, [...dates, "2026-11-06"])).toBeNull();
    expect(averageEdges(series, [...dates, "2026-11-06"])).toBeNull();
  });

  it("reads an app's hour buckets the same way", () => {
    const app = appSeries(
      [],
      [
        {
          date: "2026-11-05",
          buckets: [
            { key: "7", secs: 60 },
            { key: "21", secs: 30 },
          ],
        },
      ],
    );
    expect(widestEdges(app, ["2026-11-05"])).toEqual({ start: 7, end: 22 });
  });
});

describe("deviceShares", () => {
  it("splits the time between devices in whole percents", () => {
    expect(
      deviceShares([
        { id: "mac", secs: 700 },
        { id: "win", secs: 300 },
      ]),
    ).toEqual([
      { id: "mac", secs: 700, pct: 70 },
      { id: "win", secs: 300, pct: 30 },
    ]);
  });

  it("keeps the two biggest devices and puts the rest together", () => {
    const shares = deviceShares([
      { id: "a", secs: 100 },
      { id: "b", secs: 500 },
      { id: "c", secs: 300 },
      { id: "d", secs: 100 },
    ]);
    expect(shares?.map((s) => [s.id, s.pct])).toEqual([
      ["b", 50],
      ["c", 30],
      ["other", 20],
    ]);
  });

  it("makes the percents add up to 100", () => {
    // 三等分：33.3 各取整是 99，多出的 1 给余数最大的
    const shares = deviceShares([
      { id: "a", secs: 1 },
      { id: "b", secs: 1 },
      { id: "c", secs: 1 },
    ]);
    expect(shares?.reduce((sum, s) => sum + s.pct, 0)).toBe(100);
    // 33.6、33.6、32.8 取整是 98，多出的 2 给余数最大的 c（0.8）和 a（0.6）
    const two = deviceShares([
      { id: "a", secs: 42 },
      { id: "b", secs: 42 },
      { id: "c", secs: 41 },
    ]);
    expect(two?.map((s) => s.pct)).toEqual([34, 33, 33]);
  });

  it("is null when no device has time", () => {
    expect(deviceShares([{ id: "a", secs: 0 }])).toBeNull();
  });
});

describe("cardValue", () => {
  // 周四 15:40 看本周：本周一到周四，对比上周一到周三整天加上周四 15:40 之前
  const now = new Date(2026, 10, 5, 15, 40);
  const windows = insightWindows("week", 0, "same", now);
  const series = categorySeries(
    [
      day("2026-10-26", 3600),
      day("2026-10-27", 3600),
      day("2026-10-28", 3600),
      day("2026-10-29", 9999), // 上周四整天，不该算进去
      day("2026-11-02", 7200),
      day("2026-11-03", 3600),
      day("2026-11-04", 5400),
      day("2026-11-05", 1800),
    ],
    // 上周四：9 点 1200 秒，15 点 1200 秒（截到 40 分算 800）
    [{ date: "2026-10-29", hours: hoursOf([9, 15], 1200) }],
    null,
  );
  const input: CardInputs = {
    scope: "week",
    windows,
    comparable: true,
    series,
    edges: true,
    share: null,
    devices: null,
  };

  it("compares up to now with the previous week up to the same minute", () => {
    // 本周 2h+1h+1.5h+0.5h = 300 分；上周 3h + 1200/60 + 800/60 = 180 + 33 = 213 分
    expect(cardValue("vsPrev", input)).toEqual({ kind: "delta", minutes: 300 - 213 });
  });

  it("averages the finished days and compares them with the same days last week", () => {
    // 周一到周三：(120 + 60 + 90) / 3 = 90；上周一到周三 60
    expect(cardValue("avg", input)).toEqual({ kind: "duration", minutes: 90 });
    expect(cardValue("avgVsPrev", input)).toEqual({ kind: "delta", minutes: 30 });
  });

  it("shows no comparison when records start inside the compared stretch", () => {
    expect(cardValue("vsPrev", { ...input, comparable: false })).toBeNull();
    expect(cardValue("avgVsPrev", { ...input, comparable: false })).toBeNull();
    expect(cardValue("total", { ...input, comparable: false })).toEqual({
      kind: "duration",
      minutes: 300,
    });
  });

  it("finds the peak day in what the cards cover", () => {
    expect(cardValue("peak", input)).toEqual({ kind: "peakDay", date: "2026-11-02", minutes: 120 });
  });

  it("shows nothing while a day is not loaded, or when the stretch is empty", () => {
    const partial = categorySeries([day("2026-11-02", 7200)], [], null);
    expect(cardValue("total", { ...input, series: partial })).toBeNull();
    const empty = categorySeries(
      ["2026-11-02", "2026-11-03", "2026-11-04", "2026-11-05"].map((d) => day(d, 0)),
      [],
      null,
    );
    expect(cardValue("total", { ...input, series: empty })).toBeNull();
  });

  it("leaves out start and end times when it is told to", () => {
    expect(cardValue("span", { ...input, edges: false })).toBeNull();
  });

  it("reads the hours of the day on the day page", () => {
    const today = categorySeries(
      [day("2026-11-05", 1800)],
      [{ date: "2026-11-05", hours: hoursOf([9, 10, 14], 600) }],
      null,
    );
    const dayInput: CardInputs = {
      ...input,
      scope: "today",
      windows: insightWindows("today", 0, "same", now),
      series: today,
    };
    expect(cardValue("peak", dayInput)).toEqual({ kind: "peakHour", hour: 9, minutes: 10 });
    expect(cardValue("activeHours", dayInput)).toEqual({ kind: "hourCount", hours: 3 });
    expect(cardValue("span", dayInput)).toEqual({ kind: "edges", start: 9, end: 15 });
  });
});
