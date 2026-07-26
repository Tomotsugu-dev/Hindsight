import { describe, expect, it } from "vitest";
import { formatAxisTick } from "./duration";

describe("formatAxisTick", () => {
  it("零与分钟档", () => {
    expect(formatAxisTick(0)).toBe("0");
    expect(formatAxisTick(45)).toBe("45m");
    expect(formatAxisTick(59)).toBe("59m");
  });
  it("整小时档", () => {
    expect(formatAxisTick(60)).toBe("1h");
    expect(formatAxisTick(120)).toBe("2h");
  });
  it("0.25h 档不被 toFixed(1) 抹成 1.3h", () => {
    expect(formatAxisTick(75)).toBe("1.25h");
    expect(formatAxisTick(90)).toBe("1.5h");
    expect(formatAxisTick(990)).toBe("16.5h");
  });
});
