import { describe, expect, it } from "vitest";
import { readable, testing } from "./color";
import { formatSeconds, secondsLeft, shareLeft } from "./time";

describe("readable", () => {
  it("keeps a color that is already easy to see on black", () => {
    expect(readable("#f87171")).toBe("#f87171");
    expect(readable("#A78BFA")).toBe("#a78bfa");
  });

  it("makes a dark color lighter until it reaches 4.5:1", () => {
    for (const dark of ["#0000ff", "#1e3a8a", "#000000"]) {
      const light = readable(dark);
      const rgb = [1, 3, 5].map((i) => parseInt(light.slice(i, i + 2), 16));
      expect(testing.contrastOnBlack(rgb)).toBeGreaterThanOrEqual(4.5);
      expect(light).not.toBe(dark);
    }
  });
});

describe("timer", () => {
  const timer = {
    start: "2026-10-21T10:00:00+09:00",
    end: "2026-10-21T10:25:00+09:00",
    pausedAt: null,
    color: null,
  };
  const at = (time: string) => Date.parse(`2026-10-21T${time}+09:00`);

  it("counts down to the end and stays at 0", () => {
    expect(secondsLeft(timer, at("10:00:00"))).toBe(25 * 60);
    expect(secondsLeft(timer, at("10:24:59.5"))).toBe(1);
    expect(secondsLeft(timer, at("10:30:00"))).toBe(0);
    expect(shareLeft(timer, at("10:12:30"))).toBe(0.5);
  });

  it("stops where it was paused", () => {
    const paused = { ...timer, pausedAt: "2026-10-21T10:05:00+09:00" };
    expect(secondsLeft(paused, at("10:20:00"))).toBe(20 * 60);
  });

  it("shows hours only when there are some", () => {
    expect(formatSeconds(25 * 60)).toBe("25:00");
    expect(formatSeconds(59)).toBe("0:59");
    expect(formatSeconds(3600 + 5 * 60 + 9)).toBe("1:05:09");
  });
});
