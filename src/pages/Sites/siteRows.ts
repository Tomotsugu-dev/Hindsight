import type { SiteRow } from "../../api/hindsight";

/** Where a website's category comes from: its own rule, a parent domain's rule, or no rule. */
export type SiteRuleSource = "own" | "parent" | "none";

export function ruleSource(row: SiteRow): SiteRuleSource {
  if (row.follows !== null) return "parent";
  return row.categoryId !== null ? "own" : "none";
}

/** Websites whose host contains the search text, ignoring case. Keeps the input order. */
export function filterSites(rows: SiteRow[], search: string): SiteRow[] {
  const needle = search.trim().toLowerCase();
  if (!needle) return rows;
  return rows.filter((r) => r.host.toLowerCase().includes(needle));
}
