import { describe, expect, it } from "vitest";
import {
  autoScrollStep,
  DRAG_AUTO_SCROLL_EDGE,
  DRAG_AUTO_SCROLL_MAX_STEP,
} from "./useDragAutoScroll";

describe("autoScrollStep", () => {
  const top = 100;
  const bottom = 500;

  it("does not scroll away from the edges", () => {
    expect(autoScrollStep(300, top, bottom)).toBe(0);
  });

  it("scrolls up near the top edge", () => {
    expect(autoScrollStep(top + 1, top, bottom)).toBeLessThan(0);
  });

  it("scrolls down near the bottom edge", () => {
    expect(autoScrollStep(bottom - 1, top, bottom)).toBeGreaterThan(0);
  });

  it("scrolls faster closer to the edge", () => {
    const nearEdge = Math.abs(autoScrollStep(top + 4, top, bottom));
    const nearZone = Math.abs(autoScrollStep(top + DRAG_AUTO_SCROLL_EDGE - 2, top, bottom));
    expect(nearEdge).toBeGreaterThan(nearZone);
  });

  it("keeps scrolling at max speed outside the container", () => {
    expect(autoScrollStep(top - 40, top, bottom)).toBe(-DRAG_AUTO_SCROLL_MAX_STEP);
    expect(autoScrollStep(bottom + 40, top, bottom)).toBe(DRAG_AUTO_SCROLL_MAX_STEP);
  });

  it("stops exactly at the edge-zone boundary", () => {
    expect(autoScrollStep(top + DRAG_AUTO_SCROLL_EDGE, top, bottom)).toBe(0);
    expect(autoScrollStep(bottom - DRAG_AUTO_SCROLL_EDGE, top, bottom)).toBe(0);
  });
});
