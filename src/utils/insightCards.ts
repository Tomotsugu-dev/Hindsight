/**
 * The numbers on the stats cards, worked out from time series.
 *
 * A series holds seconds of the counted time: all of it, one category, or one app. A day or hour
 * that is missing from a series has not been loaded yet; the functions then return null.
 */

import type { CategoryTime, DetailBucket, HourSlot } from "../api/hindsight";
import {
  datesOf,
  type DateKey,
  type DateRange,
  type InsightScope,
  type InsightWindows,
  type Stretch,
} from "./insightWindows";

export type CardId =
  | "vsPrev"
  | "avg"
  | "avgVsPrev"
  | "peak"
  | "dominant"
  | "total"
  | "activeHours"
  | "span"
  | "avgSpan";

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

/** What one card shows. A card with no value shows "—". */
export type CardValue =
  | { kind: "delta"; minutes: number }
  | { kind: "duration"; minutes: number }
  | { kind: "peakHour"; hour: number; minutes: number }
  | { kind: "peakDay"; date: DateKey; minutes: number }
  | { kind: "share"; name: string; color: string; pct: number }
  | { kind: "edges"; start: number; end: number }
  | { kind: "hourCount"; hours: number };

export type ShareValue = Extract<CardValue, { kind: "share" }>;

export interface CardInputs {
  scope: InsightScope;
  windows: InsightWindows;
  /** Whether there are records from before the comparison starts; see `canCompare`. */
  comparable: boolean;
  series: Series;
  /** Whether start and end times are worked out; not for one app on the week and month pages. */
  edges: boolean;
  /** For the dominant card: the biggest part, worked out by the caller. */
  share: ShareValue | null;
}

/** Works out one card. Every card is empty while the time it covers is not loaded or is empty. */
export function cardValue(id: CardId, input: CardInputs): CardValue | null {
  const { scope, windows, series } = input;
  const currSecs = stretchSecs(series, windows.curr);
  if (currSecs === null || currSecs <= 0 || !windows.curr.days) return null;
  const { currDays, prevDays } = windows;
  // The day page covers one day; its hours give the peak, the active hours and the edges
  const dayHours = scope === "today" ? series.hours.get(windows.curr.days.from) : undefined;

  switch (id) {
    case "vsPrev": {
      const prevSecs = input.comparable ? stretchSecs(series, windows.prev) : null;
      return prevSecs === null
        ? null
        : { kind: "delta", minutes: toMinutes(currSecs) - toMinutes(prevSecs) };
    }
    case "avg": {
      const avg = currDays ? averageMinutes(series, currDays) : null;
      return avg === null ? null : { kind: "duration", minutes: avg };
    }
    case "avgVsPrev": {
      if (!input.comparable || !currDays || !prevDays) return null;
      const curr = averageMinutes(series, currDays);
      const prev = averageMinutes(series, prevDays);
      return curr === null || prev === null ? null : { kind: "delta", minutes: curr - prev };
    }
    case "peak": {
      if (scope === "today") {
        const best = dayHours ? peakHour(dayHours) : null;
        return best && { kind: "peakHour", hour: best.hour, minutes: toMinutes(best.secs) };
      }
      const best = peakDay(series, windows.curr.days);
      return best && { kind: "peakDay", date: best.date, minutes: toMinutes(best.secs) };
    }
    case "dominant":
      return input.share;
    case "total":
      return { kind: "duration", minutes: toMinutes(currSecs) };
    case "activeHours":
      return dayHours ? { kind: "hourCount", hours: activeHourCount(dayHours) } : null;
    case "span":
    case "avgSpan": {
      if (scope === "today") {
        const e = dayHours ? dayEdges(dayHours) : null;
        return e && { kind: "edges", ...e };
      }
      if (!input.edges || !currDays) return null;
      const dates = datesOf(currDays);
      const e = id === "span" ? widestEdges(series, dates) : averageEdges(series, dates);
      return e && { kind: "edges", ...e };
    }
  }
}
