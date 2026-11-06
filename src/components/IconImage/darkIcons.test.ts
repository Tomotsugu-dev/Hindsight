import { describe, expect, it, vi } from "vitest";
import { cachedVerdict, judgeIcon } from "./darkIcons";

const BLACK = new Uint8ClampedArray([0, 0, 0, 255, 0, 0, 0, 255]);

describe("judgeIcon", () => {
  it("shows no backing when the pixels cannot be read", () => {
    expect(judgeIcon("unreadable.png", () => null)).toBe(false);
  });

  it("reads the pixels of each src only once", () => {
    const read = vi.fn(() => BLACK);
    expect(judgeIcon("black.png", read)).toBe(true);
    expect(judgeIcon("black.png", read)).toBe(true);
    expect(read).toHaveBeenCalledTimes(1);
    expect(cachedVerdict("black.png")).toBe(true);
  });

  it("has no verdict for an icon it has not judged", () => {
    expect(cachedVerdict("never-loaded.png")).toBeUndefined();
  });
});
