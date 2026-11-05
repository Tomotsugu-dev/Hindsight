import { describe, expect, it } from "vitest";
import {
  dayOffsetOf,
  formatGeneratedAt,
  hourLabel,
  modelLabel,
  parseDateStr,
  toDateStr,
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
