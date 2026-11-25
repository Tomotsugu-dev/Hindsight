import { describe, expect, it } from "vitest";
import { activeSection } from "./activeSection";

const tops = [
  { id: "engine", top: 100 },
  { id: "local", top: 500 },
  { id: "cloud", top: 1100 },
];

describe("activeSection", () => {
  it("is the first section before any reaches the line", () => {
    expect(activeSection(tops, 80, false)).toBe("engine");
  });

  it("is the last section whose top reached the line", () => {
    expect(activeSection(tops, 500, false)).toBe("local");
    expect(activeSection(tops, 1099, false)).toBe("local");
  });

  it("is the last section at the bottom of the page", () => {
    expect(activeSection(tops, 600, true)).toBe("cloud");
  });

  it("is nothing without sections", () => {
    expect(activeSection([], 0, false)).toBeNull();
  });
});
