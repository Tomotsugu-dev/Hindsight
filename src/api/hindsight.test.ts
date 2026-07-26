import { afterEach, describe, expect, it, vi } from "vitest";

// hindsight.ts 顶层 import @tauri-apps/api/core;node 环境没有 Tauri IPC,
// 桩掉 invoke(本文件只测 dtoToDaySummary 纯函数,不触发任何命令调用)。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.resolve(null),
}));

import { dtoToDaySummary, type HourSegment } from "./hindsight";

describe("dtoToDaySummary", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("合法 YYYY-MM-DD 往返:解析成本地时区当日零点", () => {
    const out = dtoToDaySummary({ date: "2024-06-15", segments: [] });
    // 后端给的是"本地自然日",必须按本地时区构造——用 new Date(y,m,d) 而不是
    // Date.parse("YYYY-MM-DD")(后者按 UTC 解析,东八区会偏到前一天 08:00)
    expect(out.date.getFullYear()).toBe(2024);
    expect(out.date.getMonth()).toBe(5); // JS 月份 0 起
    expect(out.date.getDate()).toBe(15);
    expect(out.date.getHours()).toBe(0);
    expect(out.date.getMinutes()).toBe(0);
  });

  it("闰日 02-29 正常解析,不滚动", () => {
    const out = dtoToDaySummary({ date: "2024-02-29", segments: [] });
    expect(out.date.getMonth()).toBe(1);
    expect(out.date.getDate()).toBe(29);
  });

  it("segments 原样透传", () => {
    const segments: HourSegment[] = [
      { categoryId: "work", minutes: 30, secs: 1800 },
    ];
    const out = dtoToDaySummary({ date: "2024-06-15", segments });
    expect(out.segments).toBe(segments);
  });

  it("非法日期显式 throw,而不是让 Invalid Date 静默流向下游", () => {
    // 契约哨兵:格式一旦被后端改坏,要在边界处响亮报错——
    // 否则 Invalid Date 的 getDay()/getTime() 变 NaN,在下游图表里极难追踪
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(() =>
      dtoToDaySummary({ date: "not-a-date", segments: [] }),
    ).toThrow(/Invalid date from backend: not-a-date/);
    expect(() => dtoToDaySummary({ date: "", segments: [] })).toThrow(
      /Invalid date from backend/,
    );
    // throw 前先走 logError,devtools 里能按 scope 搜到
    expect(errSpy).toHaveBeenCalled();
  });

  it("已知边界:月份溢出走 JS Date 滚动语义,哨兵不拦(只拦 NaN)", () => {
    // "2024-13-05" → new Date(2024, 12, 5) 滚成 2025-01-05,不 throw。
    // 后端保证不产出这类值;这条测试钉住现状——若未来想拦滚动日期,
    // 这里会先红,提醒同步更新前后端契约注释。
    const out = dtoToDaySummary({ date: "2024-13-05", segments: [] });
    expect(out.date.getFullYear()).toBe(2025);
    expect(out.date.getMonth()).toBe(0);
    expect(out.date.getDate()).toBe(5);
  });
});
