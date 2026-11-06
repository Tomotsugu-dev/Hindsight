/**
 * Which stats cards each page shows, in what order, and how they count time.
 *
 * Stored on this machine only, one key per page, like the chart view in `statsView.ts`. A page
 * always holds 3 to 6 cards. The day page has no mode: it always compares up to the same minute.
 */

import { useSyncExternalStore } from "react";
import { logWarn } from "../lib/logger";
import type { InsightMode, InsightScope } from "../utils/insightWindows";

export type CardId =
  | "vsPrev"
  | "avg"
  | "avgVsPrev"
  | "peak"
  | "dominant"
  | "total"
  | "activeHours"
  | "devices"
  | "span"
  | "avgSpan";

export const MIN_CARDS = 3;
export const MAX_CARDS = 6;

const PERIOD_LIBRARY: readonly CardId[] = [
  "vsPrev",
  "avg",
  "avgVsPrev",
  "peak",
  "dominant",
  "total",
  "devices",
  "avgSpan",
  "span",
];

/** The cards a page can hold, in the order the card library lists them. */
export const CARD_LIBRARY: Record<InsightScope, readonly CardId[]> = {
  today: ["vsPrev", "peak", "dominant", "total", "activeHours", "span", "devices"],
  week: PERIOD_LIBRARY,
  month: PERIOD_LIBRARY,
};

export const DEFAULT_CARDS: Record<InsightScope, readonly CardId[]> = {
  today: ["vsPrev", "peak", "dominant"],
  week: ["vsPrev", "avg", "avgVsPrev", "dominant"],
  month: ["vsPrev", "avg", "avgVsPrev", "dominant"],
};

const CARDS_KEY = "hindsight.stats.cards.";
const MODE_KEY = "hindsight.stats.mode.";
const VERSION = 1;

/** Why a change was refused: the page would go below 3 cards, or above 6. */
export type EditRefusal = "min" | "max";

function isCardOf(scope: InsightScope, id: unknown): id is CardId {
  return typeof id === "string" && (CARD_LIBRARY[scope] as readonly string[]).includes(id);
}

/** Keeps the cards the page knows, once each and at most 6, then fills up to 3 from the defaults. */
export function normalizeCards(scope: InsightScope, ids: readonly unknown[]): CardId[] {
  const out: CardId[] = [];
  for (const id of ids) {
    if (out.length >= MAX_CARDS) break;
    if (isCardOf(scope, id) && !out.includes(id)) out.push(id);
  }
  for (const id of DEFAULT_CARDS[scope]) {
    if (out.length >= MIN_CARDS) break;
    if (!out.includes(id)) out.push(id);
  }
  return out;
}

/** Reads a stored card list; anything it does not recognize gives the defaults. */
export function parseCards(scope: InsightScope, raw: string | null): readonly CardId[] {
  if (raw === null) return DEFAULT_CARDS[scope];
  let stored: unknown;
  try {
    stored = JSON.parse(raw);
  } catch (e) {
    logWarn("insightCards.parse", e);
    return DEFAULT_CARDS[scope];
  }
  const { v, cards } = (stored ?? {}) as { v?: unknown; cards?: unknown };
  if (v !== VERSION || !Array.isArray(cards)) {
    logWarn("insightCards.parse", `${scope}: version ${String(v)} is not ${VERSION}`);
    return DEFAULT_CARDS[scope];
  }
  return normalizeCards(scope, cards);
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch (e) {
    logWarn("insightCards.read", e);
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch (e) {
    logWarn("insightCards.write", e);
  }
}

const readMode = (scope: "week" | "month"): InsightMode =>
  read(MODE_KEY + scope) === "rolling" ? "rolling" : "same";

const cards: Record<InsightScope, readonly CardId[]> = {
  today: parseCards("today", read(CARDS_KEY + "today")),
  week: parseCards("week", read(CARDS_KEY + "week")),
  month: parseCards("month", read(CARDS_KEY + "month")),
};
const modes: Record<InsightScope, InsightMode> = {
  today: "same",
  week: readMode("week"),
  month: readMode("month"),
};
const listeners = new Set<() => void>();

export function subscribeInsightCards(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

function save(scope: InsightScope, next: readonly CardId[]): void {
  if (next.length === cards[scope].length && next.every((id, i) => id === cards[scope][i])) return;
  cards[scope] = next;
  write(CARDS_KEY + scope, JSON.stringify({ v: VERSION, cards: next }));
  listeners.forEach((cb) => cb());
}

export function getCards(scope: InsightScope): readonly CardId[] {
  return cards[scope];
}

export function getMode(scope: InsightScope): InsightMode {
  return modes[scope];
}

/** Puts a card from the library at `at`, or at the end. */
export function addCard(scope: InsightScope, id: CardId, at?: number): EditRefusal | null {
  const list = cards[scope];
  if (!isCardOf(scope, id) || list.includes(id)) return null;
  if (list.length >= MAX_CARDS) return "max";
  const next = [...list];
  next.splice(at === undefined ? next.length : Math.min(Math.max(at, 0), next.length), 0, id);
  save(scope, next);
  return null;
}

export function removeCard(scope: InsightScope, id: CardId): EditRefusal | null {
  const list = cards[scope];
  if (!list.includes(id)) return null;
  if (list.length <= MIN_CARDS) return "min";
  save(
    scope,
    list.filter((x) => x !== id),
  );
  return null;
}

/** Moves a placed card to position `to`; a position past either end does nothing. */
export function moveCard(scope: InsightScope, id: CardId, to: number): void {
  const list = cards[scope];
  const from = list.indexOf(id);
  if (from < 0 || to < 0 || to >= list.length || to === from) return;
  const next = [...list];
  next.splice(from, 1);
  next.splice(to, 0, id);
  save(scope, next);
}

/** Puts back the default cards; the mode stays as it is. */
export function resetCards(scope: InsightScope): void {
  save(scope, DEFAULT_CARDS[scope]);
}

export function setMode(scope: "week" | "month", next: InsightMode): void {
  if (modes[scope] === next) return;
  modes[scope] = next;
  write(MODE_KEY + scope, next);
  listeners.forEach((cb) => cb());
}

export function useInsightCards(scope: InsightScope): readonly CardId[] {
  return useSyncExternalStore(subscribeInsightCards, () => cards[scope]);
}

export function useInsightMode(scope: InsightScope): InsightMode {
  return useSyncExternalStore(subscribeInsightCards, () => modes[scope]);
}
