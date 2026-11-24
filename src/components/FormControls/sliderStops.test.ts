import { describe, expect, it } from "vitest";
import { adjacentStop, nearestStop, stopPosition } from "./sliderStops";

const stops = [1, 3, 7, 30, 90, 180];

describe("stopPosition", () => {
  it("puts each stop at its index", () => {
    stops.forEach((s, i) => expect(stopPosition(s, stops)).toBe(i));
  });

  it("puts a value between two stops at a fraction between them", () => {
    expect(stopPosition(135, stops)).toBe(4.5);
  });

  it("puts values outside the stops at the ends", () => {
    expect(stopPosition(0, stops)).toBe(0);
    expect(stopPosition(1000, stops)).toBe(stops.length - 1);
  });
});

describe("nearestStop", () => {
  it("snaps a position to the nearest stop", () => {
    expect(nearestStop(4.4, stops)).toBe(90);
    expect(nearestStop(4.6, stops)).toBe(180);
  });

  it("stays within the stops", () => {
    expect(nearestStop(-1, stops)).toBe(1);
    expect(nearestStop(99, stops)).toBe(180);
  });
});

describe("adjacentStop", () => {
  it("moves to the next stop, also from a value between stops", () => {
    expect(adjacentStop(30, stops, 1)).toBe(90);
    expect(adjacentStop(100, stops, 1)).toBe(180);
    expect(adjacentStop(100, stops, -1)).toBe(90);
  });

  it("stays at either end", () => {
    expect(adjacentStop(180, stops, 1)).toBe(180);
    expect(adjacentStop(1, stops, -1)).toBe(1);
  });
});
