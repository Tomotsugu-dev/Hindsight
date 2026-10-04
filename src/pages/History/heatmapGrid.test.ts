import { describe, it, expect } from "vitest";
import {
  buildHeatmap,
  buildHeatMonth,
  clipRange,
  dayKey,
  parseDayKey,
} from "./heatmapGrid";

const YEAR_2026 = { from: "2026-01-01", to: "2026-12-31" };

describe("buildHeatmap", () => {
  it("每列一周、周一在第一行：2026-01-01 是周四，前面空三格", () => {
    const { weeks } = buildHeatmap(YEAR_2026, new Map(), "2026-01-01", "2026-12-31");
    expect(weeks).toHaveLength(53);
    expect(weeks[0].slice(0, 3)).toEqual([null, null, null]);
    expect(weeks[0][3]?.key).toBe("2026-01-01");
    // 12-31 是周四，后面三格空
    expect(weeks[52][3]?.key).toBe("2026-12-31");
    expect(weeks[52].slice(4)).toEqual([null, null, null]);
    const days = weeks.flat().filter((c) => c !== null);
    expect(days).toHaveLength(365);
  });

  it("按画出来的这段里最忙那天分四档，没有时间的是 0 档", () => {
    const secs = new Map([
      ["2026-03-02", 4 * 3600],
      ["2026-03-03", 3 * 3600],
      ["2026-03-04", 3600],
      ["2026-03-05", 30],
    ]);
    const { weeks } = buildHeatmap(YEAR_2026, secs, "2026-01-01", "2026-12-31");
    const level = (key: string) => weeks.flat().find((c) => c?.key === key)?.level;
    expect(level("2026-03-02")).toBe(4);
    expect(level("2026-03-03")).toBe(3);
    expect(level("2026-03-04")).toBe(1);
    expect(level("2026-03-05")).toBe(1);
    expect(level("2026-03-06")).toBe(0);
  });

  it("第一条记录之前、今天之后的格子标成 outside，也不算进最忙那天", () => {
    const secs = new Map([
      ["2026-02-01", 9 * 3600],
      ["2026-05-01", 3600],
    ]);
    const { weeks } = buildHeatmap(YEAR_2026, secs, "2026-03-01", "2026-10-04");
    const cell = (key: string) => weeks.flat().find((c) => c?.key === key);
    expect(cell("2026-02-01")).toMatchObject({ outside: true, secs: 0, level: 0 });
    expect(cell("2026-05-01")).toMatchObject({ outside: false, level: 4 });
    expect(cell("2026-10-05")?.outside).toBe(true);
  });

  it("一整年时，每个月 1 号所在的列给月份标签", () => {
    const { monthCols } = buildHeatmap(YEAR_2026, new Map(), "2026-01-01", "2026-12-31");
    expect(monthCols).toHaveLength(12);
    expect(monthCols[0]).toMatchObject({ col: 0 });
    expect(monthCols[0].date.getMonth()).toBe(0);
    // 02-01 是周日，还在第 4 列（0 起）
    expect(monthCols[1]).toMatchObject({ col: 4 });
  });

  it("拖出来的一段不从 1 号开始时，第一列也写上月份", () => {
    // 06-10 是周三；07-01 落在第 3 列，离得够远
    const { weeks, monthCols } = buildHeatmap(
      { from: "2026-06-10", to: "2026-07-20" },
      new Map(),
      "2026-01-01",
      "2026-12-31",
    );
    expect(weeks[0][2]?.key).toBe("2026-06-10");
    expect(monthCols.map((m) => [m.date.getMonth(), m.col])).toEqual([
      [5, 0],
      [6, 3],
    ]);
  });

  it("下个月 1 号就在旁边两列里时，第一列不写，免得挤在一起", () => {
    const { monthCols } = buildHeatmap(
      { from: "2026-06-25", to: "2026-07-20" },
      new Map(),
      "2026-01-01",
      "2026-12-31",
    );
    expect(monthCols.map((m) => m.date.getMonth())).toEqual([6]);
  });
});

describe("buildHeatMonth", () => {
  it("每行一周、周一在第一格：2026-10-01 是周四，十月排 5 行", () => {
    const weeks = buildHeatMonth(2026, 9, new Map(), "2026-01-01", "2026-12-31");
    expect(weeks).toHaveLength(5);
    expect(weeks[0].slice(0, 3)).toEqual([null, null, null]);
    expect(weeks[0][3]?.key).toBe("2026-10-01");
    expect(weeks[4][5]?.key).toBe("2026-10-31");
    expect(weeks[4][6]).toBeNull();
  });

  it("按这个月最忙那天分档，别的月份的时间不算", () => {
    const secs = new Map([
      ["2026-09-30", 10 * 3600],
      ["2026-10-05", 2 * 3600],
      ["2026-10-06", 3600],
    ]);
    const weeks = buildHeatMonth(2026, 9, secs, "2026-01-01", "2026-12-31");
    const level = (key: string) => weeks.flat().find((c) => c?.key === key)?.level;
    expect(level("2026-10-05")).toBe(4);
    expect(level("2026-10-06")).toBe(2);
  });
});

describe("clipRange", () => {
  it("取交集，不相交时为 null", () => {
    const bounds = { from: "2026-05-17", to: "2026-10-04" };
    expect(clipRange(YEAR_2026, bounds)).toEqual(bounds);
    expect(clipRange({ from: "2026-06-01", to: "2026-06-30" }, bounds)).toEqual({
      from: "2026-06-01",
      to: "2026-06-30",
    });
    expect(clipRange({ from: "2026-01-01", to: "2026-03-31" }, bounds)).toBeNull();
  });
});

describe("dayKey / parseDayKey", () => {
  it("本地日期和 YYYY-MM-DD 互转", () => {
    expect(dayKey(new Date(2026, 8, 5))).toBe("2026-09-05");
    expect(parseDayKey("2026-09-05").getTime()).toBe(new Date(2026, 8, 5).getTime());
  });
});
