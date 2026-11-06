import { describe, expect, it } from "vitest";
import { canCompare, insightWindows, periodRange } from "./insightWindows";

// 2026-11-05 是周四，15:40
const thu = new Date(2026, 10, 5, 15, 40);

describe("insightWindows: same", () => {
  it("compares today so far with yesterday up to the same minute", () => {
    const w = insightWindows("today", 0, "same", thu);
    expect(w.curr).toEqual({ days: { from: "2026-11-05", to: "2026-11-05" }, part: null });
    // 昨天只算到 15:40：15 点之前整小时，15 点那一格算 40/60
    expect(w.prev).toEqual({
      days: null,
      part: { date: "2026-11-04", hour: 15, fraction: 40 / 60 },
    });
  });

  it("compares a past day with the whole day before it", () => {
    const w = insightWindows("today", -1, "same", thu);
    expect(w.curr.days).toEqual({ from: "2026-11-04", to: "2026-11-04" });
    expect(w.prev).toEqual({ days: { from: "2026-11-03", to: "2026-11-03" }, part: null });
  });

  it("cuts last week at the same weekday and minute", () => {
    const w = insightWindows("week", 0, "same", thu);
    expect(w.curr.days).toEqual({ from: "2026-11-02", to: "2026-11-05" });
    expect(w.prev).toEqual({
      days: { from: "2026-10-26", to: "2026-10-28" },
      part: { date: "2026-10-29", hour: 15, fraction: 40 / 60 },
    });
    // 日均只算过完的周一到周三，对比上周的周一到周三
    expect(w.currDays).toEqual({ from: "2026-11-02", to: "2026-11-04" });
    expect(w.prevDays).toEqual({ from: "2026-10-26", to: "2026-10-28" });
  });

  it("has no finished days on Monday morning", () => {
    const w = insightWindows("week", 0, "same", new Date(2026, 10, 2, 10, 0));
    expect(w.prev).toEqual({ days: null, part: { date: "2026-10-26", hour: 10, fraction: 0 } });
    expect(w.currDays).toBeNull();
    expect(w.prevDays).toBeNull();
  });

  it("cuts last month at the same date and minute", () => {
    const w = insightWindows("month", 0, "same", thu);
    expect(w.curr.days).toEqual({ from: "2026-11-01", to: "2026-11-05" });
    expect(w.prev).toEqual({
      days: { from: "2026-10-01", to: "2026-10-04" },
      part: { date: "2026-10-05", hour: 15, fraction: 40 / 60 },
    });
    expect(w.currDays).toEqual({ from: "2026-11-01", to: "2026-11-04" });
    expect(w.prevDays).toEqual({ from: "2026-10-01", to: "2026-10-04" });
  });

  it("takes the whole previous month when it has no such date", () => {
    // 3 月 30 日：2 月没有 30 日，整个 2 月都算
    const w = insightWindows("month", 0, "same", new Date(2026, 2, 30, 9, 0));
    expect(w.prev).toEqual({ days: { from: "2026-02-01", to: "2026-02-28" }, part: null });
    expect(w.prevDays).toEqual({ from: "2026-02-01", to: "2026-02-28" });
  });

  it("still cuts the last day of the previous month when it has that date", () => {
    const w = insightWindows("month", 0, "same", new Date(2026, 2, 28, 9, 30));
    expect(w.prev).toEqual({
      days: { from: "2026-02-01", to: "2026-02-27" },
      part: { date: "2026-02-28", hour: 9, fraction: 0.5 },
    });
  });

  it("compares whole periods when looking at a past week", () => {
    const w = insightWindows("week", -1, "same", thu);
    expect(w.curr).toEqual({ days: { from: "2026-10-26", to: "2026-11-01" }, part: null });
    expect(w.prev).toEqual({ days: { from: "2026-10-19", to: "2026-10-25" }, part: null });
    expect(w.currDays).toEqual({ from: "2026-10-26", to: "2026-11-01" });
  });
});

describe("insightWindows: rolling", () => {
  it("takes the last 7 whole days and the 7 before them, without today", () => {
    const w = insightWindows("week", 0, "rolling", thu);
    expect(w.curr).toEqual({ days: { from: "2026-10-29", to: "2026-11-04" }, part: null });
    expect(w.prev).toEqual({ days: { from: "2026-10-22", to: "2026-10-28" }, part: null });
    expect(w.currDays).toEqual(w.curr.days);
    expect(w.prevDays).toEqual(w.prev.days);
  });

  it("takes 30 days on the month page", () => {
    const w = insightWindows("month", 0, "rolling", thu);
    expect(w.curr.days).toEqual({ from: "2026-10-06", to: "2026-11-04" });
    expect(w.prev.days).toEqual({ from: "2026-09-06", to: "2026-10-05" });
  });

  it("ends on the last day of a past period", () => {
    const w = insightWindows("week", -1, "rolling", thu);
    expect(w.curr.days).toEqual({ from: "2026-10-26", to: "2026-11-01" });
    expect(w.prev.days).toEqual({ from: "2026-10-19", to: "2026-10-25" });
  });

  it("is not used on the day page", () => {
    expect(insightWindows("today", 0, "rolling", thu)).toEqual(
      insightWindows("today", 0, "same", thu),
    );
  });
});

describe("periodRange", () => {
  it("starts the week on Monday, also on Sunday", () => {
    expect(periodRange("week", 0, new Date(2026, 10, 8, 12, 0))).toEqual({
      from: "2026-11-02",
      to: "2026-11-08",
    });
  });
});

describe("canCompare", () => {
  const w = insightWindows("week", 0, "rolling", thu); // 对比从 10-22 开始

  it("compares when records start on or before the comparison", () => {
    expect(canCompare(w, "2026-10-22")).toBe(true);
    expect(canCompare(w, "2025-01-01")).toBe(true);
  });

  it("does not compare when records start inside it, or there are none", () => {
    // 10-25 才开始记录：10-22 到 10-24 是空的，比出来是假的上升
    expect(canCompare(w, "2026-10-25")).toBe(false);
    expect(canCompare(w, null)).toBe(false);
  });
});
