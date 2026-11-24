import { describe, expect, it } from "vitest";
import { INTERVAL_MAX_SECS, INTERVAL_MIN_SECS, INTERVAL_STOPS } from "./captureInterval";

describe("screenshot interval slider", () => {
  it("goes from 5 seconds to 10 minutes", () => {
    expect(INTERVAL_MIN_SECS).toBe(5);
    expect(INTERVAL_MAX_SECS).toBe(600);
    expect(INTERVAL_STOPS[0]).toBe(INTERVAL_MIN_SECS);
    expect(INTERVAL_STOPS[INTERVAL_STOPS.length - 1]).toBe(INTERVAL_MAX_SECS);
  });

  it("has stops in increasing order", () => {
    INTERVAL_STOPS.slice(1).forEach((s, i) => expect(s).toBeGreaterThan(INTERVAL_STOPS[i]));
  });
});
