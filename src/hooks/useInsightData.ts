/**
 * Loads what the stats cards need and works out what each card shows.
 *
 * Only the cards asked for are loaded: the cards on the page, or the whole card library while
 * editing. The hours of every day are loaded only for the start and end cards.
 */

import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import {
  api,
  type AppUsage,
  type Category,
  type CategoryTime,
  type DaySummaryDto,
  type DetailBucket,
  type HourSlot,
} from "../api/hindsight";
import { superIdOf } from "../lib/segments";
import { useCategories } from "../state/categories";
import { useDeviceFilter } from "../state/deviceFilter";
import { useSuperCategories } from "../state/superCategories";
import {
  appSeries,
  cardValue,
  categorySeries,
  secsOf,
  type CardId,
  type CardValue,
  type CategoryFilter,
  type Series,
  type ShareValue,
} from "../utils/insightCards";
import {
  canCompare,
  dateKey,
  datesOf,
  daysBetween,
  firstDate,
  insightWindows,
  periodRange,
  type DateKey,
  type InsightMode,
  type InsightScope,
  type InsightWindows,
} from "../utils/insightWindows";
import {
  clearOnNewCategories,
  LIVE_TTL_MS,
  peekQuery,
  queriesVersion,
  requestQueries,
  subscribeQueries,
  type Query,
} from "./insightQueries";
import { loadAppBuckets } from "./useAppDetail";
import {
  catMinutesFromSegments,
  useSuperCategoryBreakdown,
  type BreakdownSlice,
} from "./useSuperCategoryBreakdown";

/** What the cards cover: everything, a drilled super category, a picked category, or one app. */
export type InsightFocus =
  | { kind: "all" }
  | { kind: "super"; superId: string }
  | { kind: "cat"; catId: string }
  | { kind: "app"; groupId: string; categoryId: string };

/** What the cards narrow down to: a hovered app first, then a picked category, then a drilled
 *  super category. */
export function insightFocusOf(
  app: { groupId: string; categoryId: string } | null,
  catId: string | null,
  superId: string | null,
): InsightFocus {
  if (app) return { kind: "app", groupId: app.groupId, categoryId: app.categoryId };
  if (catId !== null) return { kind: "cat", catId };
  if (superId !== null) return { kind: "super", superId };
  return { kind: "all" };
}

export interface InsightData {
  windows: InsightWindows;
  /** One entry for each card asked for; null shows "—". */
  values: Partial<Record<CardId, CardValue | null>>;
  /** The time the windows were worked out at; it moves every 30 seconds. */
  now: Date;
}

interface Args {
  scope: InsightScope;
  offset: number;
  mode: InsightMode;
  cards: readonly CardId[];
  focus: InsightFocus;
}

type Row = { date: DateKey; segments: CategoryTime[] };
type Keyed = { date: DateKey; key: string };

/** The queries to ask, in order, and the keys to read their results back. */
interface Plan {
  queries: Query[];
  earliest: string;
  /** Days of categories; the day page builds them from the hours instead. */
  rows: string | null;
  hours: Keyed[];
  appDays: string[];
  appHours: Keyed[];
  /** Every app's time over what the cards cover, for an app's share of its category. */
  apps: string | null;
}

/** The offsets of the periods that hold any day from `from` to `to`. */
function periodOffsets(scope: "week" | "month", from: DateKey, to: DateKey, now: Date): number[] {
  let off = 0;
  while (periodRange(scope, off, now).from > to) off--;
  const out: number[] = [];
  for (; periodRange(scope, off, now).to >= from; off--) out.push(off);
  return out;
}

function planQueries(
  scope: InsightScope,
  windows: InsightWindows,
  cards: readonly CardId[],
  focus: InsightFocus,
  deviceId: string | undefined,
  now: Date,
): Plan {
  const today = dateKey(now);
  const offsetOf = (date: DateKey) => daysBetween(today, date);
  const dev = deviceId ?? "";
  const curr = windows.curr.days ?? { from: today, to: today };
  const from = firstDate(windows.prev) ?? curr.from;
  const app = focus.kind === "app" ? focus : null;
  const needs = (...ids: CardId[]) => ids.some((id) => cards.includes(id));

  const queries: Query[] = [];
  const add = (key: string, live: boolean, run: () => Promise<unknown>) => {
    queries.push({ key, live, run });
    return key;
  };
  const hoursOf = (date: DateKey): Keyed => ({
    date,
    key: add(`hours|${date}|${dev}`, date >= today, () =>
      api.getDayHours(offsetOf(date), deviceId),
    ),
  });
  const appHoursOf = (groupId: string, date: DateKey): Keyed => ({
    date,
    key: add(`appHours|${groupId}|${date}|${dev}`, date >= today, () =>
      loadAppBuckets("day", offsetOf(date), groupId, deviceId),
    ),
  });

  const plan: Plan = {
    queries,
    earliest: add("earliest", true, () => api.earliestActivityDate()),
    rows: null,
    hours: [],
    appDays: [],
    appHours: [],
    apps: null,
  };
  if (scope === "today") {
    const days = [...new Set([from, curr.from])];
    plan.hours = days.map(hoursOf);
    if (app) plan.appHours = days.map((d) => appHoursOf(app.groupId, d));
  } else {
    plan.rows = add(`rows|${from}|${curr.to}|${dev}`, curr.to >= today, () =>
      api.getRangeCategoryTime(from, curr.to, deviceId),
    );
    const part = windows.prev.part?.date;
    if (part) plan.hours.push(hoursOf(part));
    if (app) {
      plan.appDays = periodOffsets(scope, from, curr.to, now).map((off) =>
        add(
          `appDays|${scope}|${off}|${app.groupId}|${dev}`,
          periodRange(scope, off, now).to >= today,
          () => loadAppBuckets(scope, off, app.groupId, deviceId),
        ),
      );
      if (part) plan.appHours.push(appHoursOf(app.groupId, part));
    }
  }
  if (app && needs("dominant")) {
    plan.apps = add(`apps|${curr.from}|${curr.to}|${dev}`, curr.to >= today, () =>
      api.getRangeApps(curr.from, curr.to, deviceId),
    );
  }
  // Last, and only for these cards: the hours of every finished day
  if (scope !== "today" && !app && needs("span", "avgSpan") && windows.currDays) {
    const have = new Set(plan.hours.map((h) => h.date));
    for (const d of datesOf(windows.currDays)) if (!have.has(d)) plan.hours.push(hoursOf(d));
  }
  return plan;
}

/** One day's time per category, added up from its hours. */
function dayOf(date: DateKey, hours: readonly HourSlot[]): Row {
  const secs = new Map<string, number>();
  for (const h of hours) {
    for (const s of h.segments) secs.set(s.categoryId, (secs.get(s.categoryId) ?? 0) + s.secs);
  }
  return { date, segments: [...secs].map(([categoryId, s]) => ({ categoryId, secs: s })) };
}

function loaded<T>(list: readonly Keyed[]): { date: DateKey; value: T }[] {
  const out: { date: DateKey; value: T }[] = [];
  for (const { date, key } of list) {
    const value = peekQuery<T>(key);
    if (value !== undefined) out.push({ date, value });
  }
  return out;
}

/** The categories whose time the cards count; null counts all of it. A drilled super category
 *  counts all its categories, also those with no time in the period. */
export function focusCategories(
  focus: InsightFocus,
  categories: readonly Category[],
  superIds: ReadonlySet<string>,
): CategoryFilter {
  if (focus.kind === "cat") return new Set([focus.catId]);
  if (focus.kind !== "super") return null;
  return new Set(
    categories
      .filter((c) => !c.builtin && superIdOf(c, superIds) === focus.superId)
      .map((c) => c.id),
  );
}

/**
 * The dominant card: the biggest super category, the biggest category in the drilled one, or the
 * hovered app's share of its own category. A picked category has nothing below it.
 *
 * `rows` are the days the cards cover; `apps` is every app's time over them.
 */
export function dominantShare(
  focus: InsightFocus,
  slices: readonly BreakdownSlice[],
  rows: readonly Row[],
  apps: readonly AppUsage[] | undefined,
  category: Category | undefined,
): ShareValue | null {
  const shareOf = (name: string, color: string, part: number, whole: number): ShareValue | null =>
    whole > 0 ? { kind: "share", name, color, pct: Math.round((part / whole) * 100) } : null;
  const secsIn = (only: CategoryFilter) =>
    rows.reduce((sum, r) => sum + secsOf(r.segments, only), 0);
  if (focus.kind === "all") {
    const top = slices[0];
    // The same total as the other cards: all the time, not only what the ring shows
    return top ? shareOf(top.name, top.color, top.minutes, Math.round(secsIn(null) / 60)) : null;
  }
  if (focus.kind === "super") {
    const slice = slices.find((s) => s.id === focus.superId);
    const top = slice?.cats[0];
    return slice && top ? shareOf(top.name, top.color, top.minutes, slice.minutes) : null;
  }
  if (focus.kind === "app") {
    const own = new Set([focus.categoryId]);
    const app = apps?.find((a) => a.groupId === focus.groupId);
    return category && app
      ? shareOf(category.name, category.color, secsOf(app.byCategory, own), secsIn(own))
      : null;
  }
  return null;
}

export function useInsightData({ scope, offset, mode, cards, focus: given }: Args): InsightData {
  const { categories, getCategory } = useCategories();
  const { supers } = useSuperCategories();
  const { selectedDeviceId: deviceId } = useDeviceFilter();
  const version = useSyncExternalStore(subscribeQueries, queriesVersion);
  // Pages build a new focus object on every render; only a new value should plan again
  const focusKey = JSON.stringify(given);
  const focus = useMemo(() => JSON.parse(focusKey) as InsightFocus, [focusKey]);

  // The previous period is cut at the current minute, so the clock moves the cut
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), LIVE_TTL_MS);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => clearOnNewCategories(categories), [categories]);

  const windows = useMemo(
    () => insightWindows(scope, offset, mode, now),
    [scope, offset, mode, now],
  );
  const plan = useMemo(
    () => planQueries(scope, windows, cards, focus, deviceId, now),
    [scope, windows, cards, focus, deviceId, now],
  );
  useEffect(() => requestQueries(plan.queries), [plan]);

  const rows = useMemo<Row[]>(() => {
    void version;
    if (plan.rows) return peekQuery<DaySummaryDto[]>(plan.rows) ?? [];
    return loaded<HourSlot[]>(plan.hours).map(({ date, value }) => dayOf(date, value));
  }, [plan, version]);

  const curr = windows.curr.days;
  const currRows = useMemo(
    () => (curr ? rows.filter((r) => r.date >= curr.from && r.date <= curr.to) : []),
    [rows, curr],
  );
  const breakdown = useSuperCategoryBreakdown(
    useMemo(() => catMinutesFromSegments(currRows), [currRows]),
  );

  const only = useMemo(
    () => focusCategories(focus, categories, new Set(supers.map((s) => s.id))),
    [focus, supers, categories],
  );

  const values = useMemo(() => {
    void version;
    const hourDays = loaded<HourSlot[]>(plan.hours).map(({ date, value }) => ({
      date,
      hours: value,
    }));

    let series: Series;
    if (focus.kind === "app") {
      const appHours = loaded<DetailBucket[]>(plan.appHours).map(({ date, value }) => ({
        date,
        buckets: value,
      }));
      // The day page has no day buckets: a day's time is the sum of its hours
      const dayBuckets =
        scope === "today"
          ? appHours.map((d) => ({ key: d.date, secs: d.buckets.reduce((s, b) => s + b.secs, 0) }))
          : plan.appDays.every((k) => peekQuery(k) !== undefined)
            ? plan.appDays.flatMap((k) => peekQuery<DetailBucket[]>(k) ?? [])
            : [];
      series = appSeries(dayBuckets, appHours);
    } else {
      series = categorySeries(rows, hourDays, only);
    }

    const share = dominantShare(
      focus,
      breakdown.slices,
      currRows,
      plan.apps ? peekQuery<AppUsage[]>(plan.apps) : undefined,
      focus.kind === "app" ? getCategory(focus.categoryId) : undefined,
    );

    const input = {
      scope,
      windows,
      comparable: canCompare(windows, peekQuery<string | null>(plan.earliest) ?? null),
      series,
      edges: scope === "today" || focus.kind !== "app",
      share,
    };
    const out: Partial<Record<CardId, CardValue | null>> = {};
    for (const id of cards) out[id] = cardValue(id, input);
    return out;
  }, [version, focus, only, plan, scope, rows, breakdown, currRows, getCategory, windows, cards]);

  return { windows, values, now };
}
