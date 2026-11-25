import { describe, expect, it } from "vitest";
import { activeSection, scrolledToEnd } from "./activeSection";

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

describe("scrolledToEnd", () => {
  it("is true at the end of a page that scrolls", () => {
    expect(scrolledToEnd(1200, 800, 2000)).toBe(true);
  });

  it("is false before the end", () => {
    expect(scrolledToEnd(600, 800, 2000)).toBe(false);
  });

  // 页面还在加载、或者内容本来就短时，所有节都看得见，不该把高亮推到最后一节
  it("is false for a page too short to scroll", () => {
    expect(scrolledToEnd(0, 800, 800)).toBe(false);
    expect(scrolledToEnd(0, 800, 300)).toBe(false);
  });
});
