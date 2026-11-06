/**
 * The cache behind the stats cards, shared by the three stats pages.
 *
 * A query that covers today goes stale after 30 seconds: it is asked again, and its old result is
 * shown until the new one arrives. A past query is kept until the day or the categories change. A
 * failed query is not asked again for 30 seconds.
 *
 * The backend answers one command at a time, so at most two queries run at once, in the order
 * asked: the page's own queries do not wait behind 30 days of hours.
 */

import { logWarn } from "../lib/logger";

export const LIVE_TTL_MS = 30_000;
const RETRY_MS = 30_000;
const MAX_ENTRIES = 400;
const MAX_RUNNING = 2;

export interface Query {
  key: string;
  /** Covers today, so its result goes stale. */
  live: boolean;
  run: () => Promise<unknown>;
}

interface Entry {
  value?: unknown;
  at: number;
  failedAt?: number;
}

const entries = new Map<string, Entry>();
const running = new Set<string>();
let waiting: Query[] = [];
/** Goes up on every clear, so results of queries sent before it are dropped. */
let generation = 0;
let cacheDay = new Date().toDateString();
let seenCategories: unknown;
let version = 0;
const listeners = new Set<() => void>();

function changed(): void {
  version++;
  listeners.forEach((cb) => cb());
}

export function subscribeQueries(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** Goes up whenever a result arrives or the cache is cleared. */
export function queriesVersion(): number {
  return version;
}

export function clearQueries(): void {
  entries.clear();
  running.clear();
  waiting = [];
  generation++;
  changed();
}

/** Clears the cache when the category list is a new one: moving an app changes every total. */
export function clearOnNewCategories(categories: unknown): void {
  if (seenCategories !== undefined && seenCategories !== categories) clearQueries();
  seenCategories = categories;
}

/** The last result of a query, also when it is stale; undefined until one arrives. */
export function peekQuery<T>(key: string): T | undefined {
  return entries.get(key)?.value as T | undefined;
}

function needsRun(q: Query, now: number): boolean {
  if (running.has(q.key)) return false;
  const e = entries.get(q.key);
  if (!e) return true;
  if (e.failedAt !== undefined) return now - e.failedAt >= RETRY_MS;
  return q.live && now - e.at > LIVE_TTL_MS;
}

function store(key: string, entry: Entry): void {
  entries.delete(key);
  entries.set(key, entry);
  while (entries.size > MAX_ENTRIES) {
    const oldest = entries.keys().next().value;
    if (oldest === undefined) break;
    entries.delete(oldest);
  }
  changed();
}

function start(q: Query): void {
  const gen = generation;
  running.add(q.key);
  q.run()
    .then(
      (value) => {
        if (gen === generation) store(q.key, { value, at: Date.now() });
      },
      (e: unknown) => {
        if (gen !== generation) return;
        logWarn("insightCards.load", e);
        // Keep the old result on screen; only the retry waits
        const old = entries.get(q.key);
        store(q.key, { value: old?.value, at: old?.at ?? 0, failedAt: Date.now() });
      },
    )
    .finally(() => {
      if (gen === generation) running.delete(q.key);
      pump();
    });
}

function pump(): void {
  while (running.size < MAX_RUNNING) {
    const q = waiting.shift();
    if (!q) return;
    if (!running.has(q.key)) start(q);
  }
}

/** Asks for the queries in this order. Queries asked for before and not started yet are dropped:
 *  only the page on screen asks, so they belong to a page or period that was left. */
export function requestQueries(queries: readonly Query[]): void {
  const today = new Date().toDateString();
  if (today !== cacheDay) {
    cacheDay = today;
    clearQueries();
  }
  const now = Date.now();
  const keys = new Set<string>();
  waiting = queries.filter((q) => !keys.has(q.key) && keys.add(q.key) && needsRun(q, now));
  pump();
}
