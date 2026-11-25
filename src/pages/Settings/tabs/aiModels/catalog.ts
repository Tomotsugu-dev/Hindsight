import type { RecommendedModel } from "../../../../api/hindsight";

/** The chips above the download list. */
export type CatalogFilter = "all" | "vision" | "reasoning" | "small";

export type CatalogSort = "default" | "size-asc" | "size-desc" | "name" | "released";

/** How many models the list shows before "Show the other N". */
export const COLLAPSED_COUNT = 8;

/** Models under this size count as small: 5 GB, in the same 1024-based GB the list shows. */
export const SMALL_BYTES = 5 * 1024 ** 3;

/** Return the bytes a model downloads: the main file and the mmproj. */
export function totalBytes(rec: RecommendedModel): number {
  return rec.mainBytes + rec.mmprojBytes;
}

/** Return bytes as GB with one decimal, the unit every model size on the page uses. */
export function gb(bytes: number): string {
  return (bytes / 1024 ** 3).toFixed(1);
}

/** Return true when the model passes the chip. */
export function matchesFilter(rec: RecommendedModel, filter: CatalogFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "vision":
      return rec.caps.includes("VISION");
    case "reasoning":
      return rec.caps.includes("REASONING") || rec.caps.includes("R1");
    case "small":
      return totalBytes(rec) < SMALL_BYTES;
  }
}

/**
 * Return the models the download list shows: not downloaded, passing the chip and the brand, in
 * the chosen order.
 *
 * `brand` "all" keeps every brand. The default order is the order of the recommended list.
 */
export function catalogList(
  recommended: RecommendedModel[],
  isInstalled: (rec: RecommendedModel) => boolean,
  filter: CatalogFilter,
  brand: string,
  sort: CatalogSort,
): RecommendedModel[] {
  const list = recommended.filter(
    (r) => !isInstalled(r) && matchesFilter(r, filter) && (brand === "all" || r.brand === brand),
  );
  if (sort === "default") return list;
  return [...list].sort((a, b) => {
    switch (sort) {
      case "size-asc":
        return totalBytes(a) - totalBytes(b);
      case "size-desc":
        return totalBytes(b) - totalBytes(a);
      case "name":
        return a.displayName.localeCompare(b.displayName);
      case "released":
        // 新的在前；没有日期的排最后；同一天保持原来的先后
        return (b.released || "0000").localeCompare(a.released || "0000");
    }
  });
}

/**
 * Return the part of the list to show: the first `COLLAPSED_COUNT` models until the user expands
 * it, plus any later model for which `keep` is true, such as one that is downloading.
 */
export function visibleCatalog(
  list: RecommendedModel[],
  expanded: boolean,
  keep: (rec: RecommendedModel) => boolean,
): RecommendedModel[] {
  if (expanded) return list;
  return list.filter((r, i) => i < COLLAPSED_COUNT || keep(r));
}
