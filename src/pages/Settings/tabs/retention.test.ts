import { describe, expect, it } from "vitest";
import { KEEP_FOREVER_DAYS } from "../../../api/hindsight";
import { stopPosition } from "../../../components/FormControls/sliderStops";
import { RETENTION_MAX_DAYS, RETENTION_STOPS } from "./retention";

describe("screenshot retention slider", () => {
  it("goes from one day to 2 years, then forever", () => {
    expect(RETENTION_MAX_DAYS).toBe(730);
    expect(RETENTION_STOPS[0]).toBe(1);
    const n = RETENTION_STOPS.length;
    expect(RETENTION_STOPS[n - 2]).toBe(RETENTION_MAX_DAYS);
    expect(RETENTION_STOPS[n - 1]).toBe(KEEP_FOREVER_DAYS);
  });

  it("has stops in increasing order", () => {
    RETENTION_STOPS.slice(1).forEach((s, i) => expect(s).toBeGreaterThan(RETENTION_STOPS[i]));
  });

  it("puts a typed number of days between the stops around it", () => {
    const at = stopPosition(100, RETENTION_STOPS);
    expect(at).toBeGreaterThan(RETENTION_STOPS.indexOf(90));
    expect(at).toBeLessThan(RETENTION_STOPS.indexOf(180));
  });
});
