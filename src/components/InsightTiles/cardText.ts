/**
 * The words on the stats cards: names in the card library, labels under the values, what each
 * card covers, and the line above the cards that says which stretch of time they cover.
 */

import type { TFunction } from "i18next";
import type { InsightFocus } from "../../hooks/useInsightData";
import type { CardId, CardValue } from "../../utils/insightCards";
import {
  addDays,
  dateKey,
  parseDateKey,
  type DateKey,
  type DateRange,
  type InsightMode,
  type InsightScope,
  type InsightWindows,
} from "../../utils/insightWindows";

const K = "components.insightCards";
const DOW_KEYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"] as const;
const ROLLING_DAYS = { today: 0, week: 7, month: 30 } as const;

/** What decides the words on a page. */
export interface CardContext {
  scope: InsightScope;
  /** "rolling" only on the week and month pages. */
  mode: InsightMode;
  /** The period on screen holds today, so the previous one is cut at the same point. */
  live: boolean;
  focus: InsightFocus["kind"];
}

export function cardContext(
  scope: InsightScope,
  mode: InsightMode,
  windows: InsightWindows,
  now: Date,
  focus: InsightFocus,
): CardContext {
  const rolling = scope !== "today" && mode === "rolling";
  return {
    scope,
    mode: rolling ? "rolling" : "same",
    live: !rolling && windows.curr.days?.to === dateKey(now),
    focus: focus.kind,
  };
}

const pad = (n: number) => String(n).padStart(2, "0");

/** A whole hour, as on the peak card: "09:00". */
export const hourText = (hour: number) => `${pad(hour)}:00`;

export function dateText(t: TFunction, key: DateKey): string {
  const d = parseDateKey(key);
  return t(`${K}.date`, { month: d.getMonth() + 1, day: d.getDate() });
}

export function weekdayText(t: TFunction, key: DateKey): string {
  return t(`week.dow.${DOW_KEYS[parseDateKey(key).getDay()]}`);
}

function rangeText(t: TFunction, range: DateRange | null): string {
  if (!range) return "";
  if (range.from === range.to) return dateText(t, range.from);
  return t(`${K}.range`, { from: dateText(t, range.from), to: dateText(t, range.to) });
}

/** The line above the cards: which stretch they cover, and what they compare it with. */
export function scopeNote(
  t: TFunction,
  ctx: CardContext,
  windows: InsightWindows,
  now: Date,
): string {
  const { curr, prev } = windows;
  if (!curr.days) return "";
  const time = `${now.getHours()}:${pad(now.getMinutes())}`;
  if (ctx.mode === "rolling") {
    const params = {
      count: ROLLING_DAYS[ctx.scope],
      curr: rangeText(t, curr.days),
      prev: rangeText(t, prev.days),
    };
    // A past period ends on its own last day, not yesterday
    const recent = curr.days.to === addDays(dateKey(now), -1);
    return t(`${K}.note.${recent ? "rolling" : "rollingPast"}`, params);
  }
  if (!ctx.live) {
    return t(`${K}.note.whole`, { curr: rangeText(t, curr.days), prev: rangeText(t, prev.days) });
  }
  if (ctx.scope === "today") return t(`${K}.note.todayLive`, { time });
  const from = dateText(t, curr.days.from);
  // The previous month has no such day: all of it counts
  if (!prev.part) return t(`${K}.note.liveWholePrev`, { from, prev: rangeText(t, prev.days) });
  if (ctx.scope === "week") {
    return t(`${K}.note.weekLive`, { weekday: weekdayText(t, prev.part.date), time });
  }
  return t(`${K}.note.monthLive`, {
    from,
    prevFrom: dateText(t, prev.days?.from ?? prev.part.date),
    prevCut: dateText(t, prev.part.date),
    time,
  });
}

/** The card's name in the card library. */
export function cardName(t: TFunction, id: CardId, scope: InsightScope): string {
  if (id === "vsPrev" && scope === "today") return t(`${K}.name.vsPrevToday`);
  if (id === "peak") return t(`${K}.name.${scope === "today" ? "peakHour" : "peakDay"}`);
  return t(`${K}.name.${id}`);
}

/** The label under the card's value. */
export function cardLabel(
  t: TFunction,
  id: CardId,
  ctx: CardContext,
  value: CardValue | null | undefined,
  duration: (minutes: number) => string,
): string {
  const count = ROLLING_DAYS[ctx.scope];
  const rolling = ctx.mode === "rolling";
  const period = rolling ? "rolling" : `${ctx.scope}${ctx.live ? "Live" : ""}`;
  switch (id) {
    case "vsPrev":
    case "avgVsPrev":
      return t(`${K}.label.${id}.${period}`, { count });
    case "avg":
    case "total":
      return rolling ? t(`${K}.label.${id}Rolling`, { count }) : t(`${K}.label.${id}`);
    case "peak":
      return value?.kind === "peakHour" || value?.kind === "peakDay"
        ? t(`${K}.label.peakWith`, { duration: duration(value.minutes) })
        : t(`${K}.label.peak`);
    case "dominant": {
      const kind =
        ctx.focus === "all" ? "dominant" : ctx.focus === "app" ? "appShare" : "composition";
      return t(`${K}.label.${kind}`);
    }
    case "devices":
      return value?.kind === "devices"
        ? value.shares.map((s) => `${s.name ?? t(`${K}.other`)} ${s.pct}%`).join(" · ")
        : t(`${K}.label.devices`);
    default:
      return t(`${K}.label.${id}`);
  }
}

/** What the card covers, for the card library. */
export function cardDesc(t: TFunction, id: CardId, ctx: CardContext): string {
  const count = ROLLING_DAYS[ctx.scope];
  const variant = ctx.mode === "rolling" ? "rolling" : ctx.scope;
  switch (id) {
    case "dominant":
    case "total":
    case "devices":
      return t(`${K}.desc.${id}`, { span: t(`${K}.spanOf.${variant}`, { count }) });
    case "activeHours":
      return t(`${K}.desc.activeHours`);
    default:
      return t(`${K}.desc.${id}.${variant}`, { count });
  }
}
