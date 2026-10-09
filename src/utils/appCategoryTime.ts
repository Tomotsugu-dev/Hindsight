import type { CategoryTime } from "../api/hindsight";

/** An app's seconds in the given categories. */
export function secsIn(
  byCategory: CategoryTime[] | undefined,
  catIds: ReadonlySet<string>,
): number {
  let secs = 0;
  for (const c of byCategory ?? []) {
    if (catIds.has(c.categoryId)) secs += c.secs;
  }
  return secs;
}

/**
 * Filters apps by category. Keeps the apps with time in `catIds`, shows only that part of their
 * time, and sorts by it, most first. The minutes of the result add up to the categories' total.
 *
 * Website rules can split a browser's time: Chrome 35 min = Browsing 25 + Video 10 shows as
 * 10 min under Video.
 */
export function appsInCategories<T extends { minutes: number; byCategory?: CategoryTime[] }>(
  apps: T[],
  catIds: ReadonlySet<string>,
): T[] {
  return apps
    .map((app) => ({ app, secs: secsIn(app.byCategory, catIds) }))
    .filter((x) => x.secs > 0)
    .sort((a, b) => b.secs - a.secs)
    .map(({ app, secs }) => ({ ...app, minutes: Math.round(secs / 60) }));
}
