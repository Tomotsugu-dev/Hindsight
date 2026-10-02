import type { HourSegment } from "../api/hindsight";

/** Total seconds of one bar in the Daily, Weekly and Monthly bar charts. A segment's share of
 * the bar is `seg.secs / barSecs(segments)`. */
export function barSecs(segments: HourSegment[]): number {
  return segments.reduce((s, x) => s + x.secs, 0);
}

/** Total minutes of one bar. Seconds are added up first and rounded once, so the bar matches
 * totals computed from `secs` elsewhere. */
export function barMinutes(segments: HourSegment[]): number {
  return Math.round(barSecs(segments) / 60);
}
