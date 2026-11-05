import { describe, expect, it } from "vitest";
import {
  compactDuration,
  dayOffsetOf,
  formatGeneratedAt,
  heatLevel,
  hourLabel,
  modelLabel,
  parseDateStr,
  toDateStr,
  weekDayKind,
} from "./reportFormat";

describe("modelLabel", () => {
  it("去掉本地模型文件名的 .gguf", () => {
    expect(modelLabel("qwen2.5-vl-3b-instruct-q4_k_m.gguf")).toBe("qwen2.5-vl-3b-instruct-q4_k_m");
    expect(modelLabel("Model.GGUF")).toBe("Model");
  });

  it("云端模型名原样", () => {
    expect(modelLabel("deepseek-chat")).toBe("deepseek-chat");
  });
});

describe("toDateStr / parseDateStr", () => {
  it("按本地时间来回转换", () => {
    expect(toDateStr(parseDateStr("2026-11-04"))).toBe("2026-11-04");
    expect(toDateStr(new Date(2026, 0, 9, 23, 59))).toBe("2026-01-09");
  });
});

describe("formatGeneratedAt", () => {
  const lateNight = new Date(2026, 10, 4, 23, 40).toISOString();

  it("生成时间就在查看的那天：只写时分", () => {
    const text = formatGeneratedAt(lateNight, "en-US", "2026-11-04");
    expect(text).not.toBeNull();
    expect(text).not.toContain("11/4");
  });

  it("隔天生成，或没给日期：带上月日", () => {
    expect(formatGeneratedAt(lateNight, "en-US", "2026-11-03")).toContain("11/4");
    expect(formatGeneratedAt(lateNight, "en-US")).toContain("11/4");
  });

  it("读不出来的时间返回 null", () => {
    expect(formatGeneratedAt("not a time", "en-US")).toBeNull();
  });
});

describe("hourLabel", () => {
  it("补零，24 点照写", () => {
    expect(hourLabel(6)).toBe("06:00");
    expect(hourLabel(24)).toBe("24:00");
  });
});

describe("compactDuration", () => {
  it("不到一小时只写分钟，整点不写 0m", () => {
    expect(compactDuration(0)).toBe("0m");
    expect(compactDuration(45)).toBe("45m");
    expect(compactDuration(360)).toBe("6h");
    expect(compactDuration(380)).toBe("6h 20m");
  });
});

describe("heatLevel", () => {
  it("没有活动是 0 级，有一点活动就从 1 级起", () => {
    expect(heatLevel(0, 600)).toBe(0);
    expect(heatLevel(1, 600)).toBe(1);
  });

  it("按这周最忙的那天分级，最忙的那天是 5 级", () => {
    // 一周都在 7 到 15 小时之间，颜色也要分得开
    expect(heatLevel(935, 935)).toBe(5);
    expect(heatLevel(679, 935)).toBe(4);
    expect(heatLevel(452, 935)).toBe(2);
  });

  it("很闲的一周按 4 小时算，不会被染成最深", () => {
    expect(heatLevel(60, 60)).toBe(1);
    expect(heatLevel(120, 120)).toBe(3);
    expect(heatLevel(240, 240)).toBe(5);
  });
});

describe("dayOffsetOf", () => {
  const now = new Date(2026, 10, 5, 9, 30);

  it("今天是 0，往前数是负数", () => {
    expect(dayOffsetOf("2026-11-05", now)).toBe(0);
    expect(dayOffsetOf("2026-11-04", now)).toBe(-1);
    expect(dayOffsetOf("2026-10-29", now)).toBe(-7);
  });

  it("跨夏令时也按日历天数算", () => {
    // 美国 2026-11-01 回拨一小时；本地时区不同也不影响整天数
    expect(dayOffsetOf("2026-10-31", now)).toBe(-5);
  });

  it("没给、读不出来、在今天之后都当今天", () => {
    expect(dayOffsetOf(null, now)).toBe(0);
    expect(dayOffsetOf("garbage", now)).toBe(0);
    expect(dayOffsetOf("2026-11-09", now)).toBe(0);
  });
});

describe("weekDayKind", () => {
  const today = "2026-11-04";

  it("今天之后的算未来，哪怕标着有日报", () => {
    expect(weekDayKind({ date: "2026-11-05", hasDaily: true }, today)).toBe("future");
  });

  it("今天和之前看有没有日报", () => {
    expect(weekDayKind({ date: today, hasDaily: true }, today)).toBe("daily");
    expect(weekDayKind({ date: "2026-11-02", hasDaily: false }, today)).toBe("noDaily");
  });
});
