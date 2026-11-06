/**
 * The numbers on the stats cards, worked out from time series.
 *
 * A series holds seconds of the counted time: all of it, one category, or one app. A day or hour
 * that is missing from a series has not been loaded yet; the functions then return null.
 */

import type { CategoryTime, DetailBucket, HourSlot } from "../api/hindsight";
import { datesOf, type DateKey, type DateRange, type Stretch } from "./insightWindows";

export interface Series {
  /** Seconds on each whole day. */
  days: ReadonlyMap<DateKey, number>;
  /** Seconds in each of the 24 hours, for the days whose hours were loaded. */
  hours: ReadonlyMap<DateKey, readonly number[]>;
}

/** The categories whose time counts; null counts all of it. */
export type CategoryFilter = ReadonlySet<string> | null;

/** Start and end in whole hours: 9 and 16 mean the records fall between 9:00 and 16:00. */
export interface Edges {
  start: number;
  end: number;
}

export interface Share {
  /** A device id, or "other" for the devices after the first two. */
  id: string;
  secs: number;
  /** Whole percents; the shares add up to 100. */
  pct: number;
}

export const OTHER_SHARE = "other";

export function secsOf(segments: readonly CategoryTime[], only: CategoryFilter): number {
  let secs = 0;
  for (const s of segments) if (only === null || only.has(s.categoryId)) secs += s.secs;
  return secs;
}

function hourArray(slots: readonly { hour: number; secs: number }[]): number[] {
  const out = new Array<number>(24).fill(0);
  for (const s of slots) if (s.hour >= 0 && s.hour < 24) out[s.hour] += s.secs;
  return out;
}

export function categorySeries(
  days: readonly { date: DateKey; segments: readonly CategoryTime[] }[],
  hourDays: readonly { date: DateKey; hours: readonly HourSlot[] }[],
  only: CategoryFilter,
): Series {
  return {
    days: new Map(days.map((d) => [d.date, secsOf(d.segments, only)])),
    hours: new Map(
      hourDays.map((d) => [
        d.date,
        hourArray(d.hours.map((h) => ({ hour: h.hour, secs: secsOf(h.segments, only) }))),
      ]),
    ),
  };
}

/** An app's series: day buckets keyed "YYYY-MM-DD", hour buckets keyed "0" to "23". */
export function appSeries(
  dayBuckets: readonly DetailBucket[],
  hourDays: readonly { date: DateKey; buckets: readonly DetailBucket[] }[],
): Series {
  return {
    days: new Map(dayBuckets.map((b) => [b.key, b.secs])),
    hours: new Map(
      hourDays.map((d) => [
        d.date,
        hourArray(d.buckets.map((b) => ({ hour: parseInt(b.key, 10), secs: b.secs }))),
      ]),
    ),
  };
}

export function stretchSecs(series: Series, stretch: Stretch): number | null {
  let secs = 0;
  if (stretch.days) {
    for (const d of datesOf(stretch.days)) {
      const v = series.days.get(d);
      if (v === undefined) return null;
      secs += v;
    }
  }
  if (stretch.part) {
    const hours = series.hours.get(stretch.part.date);
    if (!hours) return null;
    for (let h = 0; h < stretch.part.hour; h++) secs += hours[h];
    secs += hours[stretch.part.hour] * stretch.part.fraction;
  }
  return secs;
}

export const rangeSecs = (series: Series, range: DateRange): number | null =>
  stretchSecs(series, { days: range, part: null });

export function toMinutes(secs: number): number {
  return Math.round(secs / 60);
}

/** Average minutes per day over a range, days without records included. */
export function averageMinutes(series: Series, range: DateRange): number | null {
  const secs = rangeSecs(series, range);
  return secs === null ? null : Math.round(secs / 60 / datesOf(range).length);
}

/** The day with the most time; the earlier one on a tie. */
export function peakDay(series: Series, range: DateRange): { date: DateKey; secs: number } | null {
  let best: { date: DateKey; secs: number } | null = null;
  for (const date of datesOf(range)) {
    const secs = series.days.get(date) ?? 0;
    if (secs > 0 && (!best || secs > best.secs)) best = { date, secs };
  }
  return best;
}

/** The hour with the most time; the earlier one on a tie. */
export function peakHour(hours: readonly number[]): { hour: number; secs: number } | null {
  let best: { hour: number; secs: number } | null = null;
  for (let hour = 0; hour < hours.length; hour++) {
    const secs = hours[hour];
    if (secs > 0 && (!best || secs > best.secs)) best = { hour, secs };
  }
  return best;
}

export function dayEdges(hours: readonly number[]): Edges | null {
  const first = hours.findIndex((s) => s > 0);
  if (first < 0) return null;
  let last = hours.length - 1;
  while (hours[last] <= 0) last--;
  return { start: first, end: last + 1 };
}

/** The earliest start and latest end over the days; null until every day's hours are loaded. */
export function widestEdges(series: Series, dates: readonly DateKey[]): Edges | null {
  let out: Edges | null = null;
  for (const date of dates) {
    const hours = series.hours.get(date);
    if (!hours) return null;
    const e = dayEdges(hours);
    if (e) out = out ? { start: Math.min(out.start, e.start), end: Math.max(out.end, e.end) } : e;
  }
  return out;
}

/** The average start and end over the days with records, each rounded to the hour. */
export function averageEdges(series: Series, dates: readonly DateKey[]): Edges | null {
  const edges: Edges[] = [];
  for (const date of dates) {
    const hours = series.hours.get(date);
    if (!hours) return null;
    const e = dayEdges(hours);
    if (e) edges.push(e);
  }
  if (edges.length === 0) return null;
  const mean = (pick: (e: Edges) => number) =>
    Math.round(edges.reduce((sum, e) => sum + pick(e), 0) / edges.length);
  return { start: mean((e) => e.start), end: mean((e) => e.end) };
}

export function activeHourCount(hours: readonly number[]): number {
  return hours.filter((s) => s > 0).length;
}

/** The two biggest devices and the rest as one share; null when no device has time. */
export function deviceShares(totals: readonly { id: string; secs: number }[]): Share[] | null {
  const sorted = totals.filter((t) => t.secs > 0).sort((a, b) => b.secs - a.secs);
  const total = sorted.reduce((sum, t) => sum + t.secs, 0);
  if (total <= 0) return null;
  const parts =
    sorted.length > 2
      ? [
          ...sorted.slice(0, 2),
          { id: OTHER_SHARE, secs: sorted.slice(2).reduce((sum, t) => sum + t.secs, 0) },
        ]
      : sorted;
  // Round down, then give the points left over to the biggest remainders, so they add up to 100
  const exact = parts.map((p) => (p.secs / total) * 100);
  const pcts = exact.map(Math.floor);
  const order = exact
    .map((x, i) => ({ i, rest: x - Math.floor(x) }))
    .sort((a, b) => b.rest - a.rest);
  const left = 100 - pcts.reduce((a, b) => a + b, 0);
  for (let k = 0; k < left; k++) pcts[order[k].i]++;
  return parts.map((p, i) => ({ id: p.id, secs: p.secs, pct: pcts[i] }));
}
