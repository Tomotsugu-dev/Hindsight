/**
 * The stretches of time the stats cards cover, and the stretches they compare with.
 *
 * A day runs from 0:00 to 24:00 local time. Weeks start on Monday, as on the backend.
 */

export type InsightScope = "today" | "week" | "month";
/** `same`: up to now, compared with the previous period up to the same point.
 *  `rolling`: the last 7 or 30 whole days, compared with the 7 or 30 days before. */
export type InsightMode = "same" | "rolling";

/** A local date, "YYYY-MM-DD". Keys sort in date order. */
export type DateKey = string;

/** Whole days, both ends included. */
export interface DateRange {
  from: DateKey;
  to: DateKey;
}

/** The start of one day: the hours before `hour` in full, plus `fraction` of hour `hour`. */
export interface PartDay {
  date: DateKey;
  hour: number;
  fraction: number;
}

/** Whole days, then maybe the start of the day after them. */
export interface Stretch {
  days: DateRange | null;
  part: PartDay | null;
}

export interface InsightWindows {
  /** What the cards cover. Today counts as a whole day: it only has data up to now. */
  curr: Stretch;
  /** What the comparison cards compare with. */
  prev: Stretch;
  /** The finished days of `curr`, for averages and start and end times; null when none. */
  currDays: DateRange | null;
  /** The days whose average `currDays` is compared with; null when none. */
  prevDays: DateRange | null;
}

const ROLLING_DAYS: Record<Exclude<InsightScope, "today">, number> = { week: 7, month: 30 };

export function dateKey(d: Date): DateKey {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** Local midnight of a date key. */
export function parseDateKey(key: DateKey): Date {
  const [y, m, d] = key.split("-").map((s) => parseInt(s, 10));
  return new Date(y, m - 1, d);
}

export function addDays(key: DateKey, n: number): DateKey {
  const d = parseDateKey(key);
  return dateKey(new Date(d.getFullYear(), d.getMonth(), d.getDate() + n));
}

/** Whole days from `from` to `to`; 0 when they are the same day. */
export function daysBetween(from: DateKey, to: DateKey): number {
  return Math.round((parseDateKey(to).getTime() - parseDateKey(from).getTime()) / 86_400_000);
}

export function datesOf(range: DateRange): DateKey[] {
  const out: DateKey[] = [];
  for (let d = range.from; d <= range.to; d = addDays(d, 1)) out.push(d);
  return out;
}

/** The days of the period `offset` periods from the one that holds `now`. */
export function periodRange(scope: InsightScope, offset: number, now: Date): DateRange {
  const y = now.getFullYear();
  const m = now.getMonth();
  const d = now.getDate();
  if (scope === "today") {
    const day = dateKey(new Date(y, m, d + offset));
    return { from: day, to: day };
  }
  if (scope === "week") {
    const sinceMonday = (now.getDay() + 6) % 7;
    const monday = new Date(y, m, d - sinceMonday + offset * 7);
    return { from: dateKey(monday), to: addDays(dateKey(monday), 6) };
  }
  return { from: dateKey(new Date(y, m + offset, 1)), to: dateKey(new Date(y, m + offset + 1, 0)) };
}

/** Works out both stretches for a page, its offset and its mode, at `now`. */
export function insightWindows(
  scope: InsightScope,
  offset: number,
  mode: InsightMode,
  now: Date,
): InsightWindows {
  const today = dateKey(now);
  const yesterday = addDays(today, -1);
  const period = periodRange(scope, offset, now);

  if (mode === "rolling" && scope !== "today") {
    const n = ROLLING_DAYS[scope];
    const end = period.to < yesterday ? period.to : yesterday;
    const curr = { from: addDays(end, 1 - n), to: end };
    const prev = { from: addDays(end, 1 - 2 * n), to: addDays(end, -n) };
    return {
      curr: { days: curr, part: null },
      prev: { days: prev, part: null },
      currDays: curr,
      prevDays: prev,
    };
  }

  const prevPeriod = periodRange(scope, offset - 1, now);
  if (today < period.from || today > period.to) {
    return {
      curr: { days: period, part: null },
      prev: { days: prevPeriod, part: null },
      currDays: period,
      prevDays: prevPeriod,
    };
  }

  // The period is still going: the previous one is cut at the same point
  const done = daysBetween(period.from, today);
  const cut = addDays(prevPeriod.from, done);
  const lastPrevDay = addDays(prevPeriod.from, done - 1);
  return {
    curr: { days: { from: period.from, to: today }, part: null },
    // The previous month is shorter and has no such day: all of it counts
    prev:
      cut > prevPeriod.to
        ? { days: prevPeriod, part: null }
        : {
            days: done > 0 ? { from: prevPeriod.from, to: addDays(cut, -1) } : null,
            part: { date: cut, hour: now.getHours(), fraction: now.getMinutes() / 60 },
          },
    currDays: done > 0 ? { from: period.from, to: yesterday } : null,
    prevDays:
      done > 0
        ? { from: prevPeriod.from, to: lastPrevDay < prevPeriod.to ? lastPrevDay : prevPeriod.to }
        : null,
  };
}

/** The first day `stretch` touches. */
export function firstDate(stretch: Stretch): DateKey | null {
  return stretch.days?.from ?? stretch.part?.date ?? null;
}

/** Whether there is a record from before the comparison starts: without one, part of it is
 *  empty only because nothing was recorded yet, and the comparison would show a rise. */
export function canCompare(windows: InsightWindows, earliest: DateKey | null): boolean {
  const start = firstDate(windows.prev);
  return earliest !== null && start !== null && earliest <= start;
}
