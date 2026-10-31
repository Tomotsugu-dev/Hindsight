import type { AppUsage, Category, CategoryTime, SuperCategory } from "../api/hindsight";
import type { BarLevel } from "../state/statsView";

/** Total seconds of one bar in the Daily, Weekly and Monthly bar charts. A segment's share of
 * the bar is `seg.secs / barSecs(segments)`. */
export function barSecs(segments: CategoryTime[]): number {
  return segments.reduce((s, x) => s + x.secs, 0);
}

/** Total minutes of one bar. Seconds are added up first and rounded once, so the bar matches
 * totals computed from `secs` elsewhere. */
export function barMinutes(segments: CategoryTime[]): number {
  return Math.round(barSecs(segments) / 60);
}

/** Id of the Ungrouped bucket, which holds categories that are in no super category. */
export const UNGROUPED_ID = "__orphan__";
/** Color of the Ungrouped bucket: a warm gray. */
export const UNGROUPED_COLOR = "#a8a29e";

/** Returns the id of the super category that a category is in, or {@link UNGROUPED_ID}.
 *
 * A category also counts as ungrouped when its super category was deleted, or when the
 * category itself cannot be found. */
export function superIdOf(category: Category | undefined, superIds: ReadonlySet<string>): string {
  const id = category?.superCategoryId;
  return id && superIds.has(id) ? id : UNGROUPED_ID;
}

/** Merges the category segments of one bar into one part per super category.
 *
 * Parts follow the order of `supers`, and Ungrouped comes last, so it sits on top of the bar.
 * No time is dropped: the parts add up to the same seconds as the segments. */
export function segmentsBySuper(
  segments: CategoryTime[],
  categories: Category[],
  supers: SuperCategory[],
): { id: string; secs: number }[] {
  const catById = new Map(categories.map((c) => [c.id, c]));
  const superIds = new Set(supers.map((s) => s.id));
  const secsById = new Map<string, number>();
  for (const seg of segments) {
    const id = superIdOf(catById.get(seg.categoryId), superIds);
    secsById.set(id, (secsById.get(id) ?? 0) + seg.secs);
  }
  return [...supers.map((s) => s.id), UNGROUPED_ID]
    .map((id) => ({ id, secs: secsById.get(id) ?? 0 }))
    .filter((part) => part.secs > 0);
}

/** Time and apps of one row in the category ranking. */
export interface LevelRank {
  secs: number;
  /** The `iconProcess` of each app with time in this row, most time first. Icons are looked up
   * by this name; an app group's display name may not match it. */
  iconProcesses: string[];
}

/** Adds up the time of each category, or of each super category, for the category ranking.
 *
 * At the super category level the keys are super category ids, and Ungrouped collects the rest
 * as in {@link segmentsBySuper}. An app with time in several categories of one super category is
 * listed once in that row, with its time added up. */
export function rankByLevel(
  level: BarLevel,
  sources: { segments: CategoryTime[] }[],
  apps: AppUsage[],
  categories: Category[],
  supers: SuperCategory[],
): Map<string, LevelRank> {
  const catById = new Map(categories.map((c) => [c.id, c]));
  const superIds = new Set(supers.map((s) => s.id));
  const keyOf = (categoryId: string) =>
    level === "supers" ? superIdOf(catById.get(categoryId), superIds) : categoryId;

  const secsByKey = new Map<string, number>();
  for (const src of sources) {
    for (const seg of src.segments) {
      const key = keyOf(seg.categoryId);
      secsByKey.set(key, (secsByKey.get(key) ?? 0) + seg.secs);
    }
  }

  type AppSecs = { iconProcess: string; secs: number };
  // Keyed by app group inside each row, so an app counts once per row.
  const appsByKey = new Map<string, Map<string, AppSecs>>();
  for (const a of apps) {
    for (const c of a.byCategory) {
      const key = keyOf(c.categoryId);
      const row = appsByKey.get(key) ?? new Map<string, AppSecs>();
      const secs = (row.get(a.groupId)?.secs ?? 0) + c.secs;
      row.set(a.groupId, { iconProcess: a.iconProcess, secs });
      appsByKey.set(key, row);
    }
  }

  const ranks = new Map<string, LevelRank>();
  for (const [key, secs] of secsByKey) {
    const rowApps = [...(appsByKey.get(key)?.values() ?? [])].sort((x, y) => y.secs - x.secs);
    ranks.set(key, { secs, iconProcesses: rowApps.map((x) => x.iconProcess) });
  }
  return ranks;
}

/** One entry in the legend under a bar chart. */
export interface LegendItem {
  id: string;
  name: string;
  color: string;
}

/** Returns the legend entries for a bar chart colored by `level`.
 *
 * By category, every category except Hidden is listed. By super category, the super categories
 * are listed in order, followed by Ungrouped when some category is in no super category. */
export function legendItems(
  level: BarLevel,
  categories: Category[],
  supers: SuperCategory[],
  ungroupedName: string,
): LegendItem[] {
  const shown = categories.filter((c) => c.id !== "hidden");
  if (level === "cats") {
    return shown.map(({ id, name, color }) => ({ id, name, color }));
  }
  const superIds = new Set(supers.map((s) => s.id));
  const items = supers.map(({ id, name, color }) => ({ id, name, color }));
  if (shown.some((c) => superIdOf(c, superIds) === UNGROUPED_ID)) {
    items.push({ id: UNGROUPED_ID, name: ungroupedName, color: UNGROUPED_COLOR });
  }
  return items;
}
