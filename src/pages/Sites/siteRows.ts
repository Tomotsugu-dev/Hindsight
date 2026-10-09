import type { SiteRow } from "../../api/hindsight";

/** Where a website's category comes from: its own rule, a parent domain's rule, or no rule. */
export type SiteRuleSource = "own" | "parent" | "none";

export function ruleSource(row: SiteRow): SiteRuleSource {
  if (row.follows !== null) return "parent";
  return row.categoryId !== null ? "own" : "none";
}

/** A website and the subdomains listed under it. */
export interface SiteGroup {
  root: SiteRow;
  /** Subdomains of `root`, in input order */
  children: SiteRow[];
  /** How many children count toward a different category than `root` */
  otherCategoryCount: number;
}

/** The shortest parent domain of `host` that is in `hosts`, or `host` itself when none is. */
function groupRoot(host: string, hosts: ReadonlySet<string>): string {
  // An IP address has no parent domains.
  if (/^[\d.]+$/.test(host)) return host;
  const labels = host.split(".");
  for (let i = labels.length - 2; i >= 1; i--) {
    const parent = labels.slice(i).join(".");
    if (hosts.has(parent)) return parent;
  }
  return host;
}

/**
 * Puts each website under the shortest parent domain that is itself in the list:
 * live.bilibili.com goes under bilibili.com. A website whose parent domains are not in the
 * list stays on its own; nothing is guessed from the domain name.
 *
 * Groups are ordered by the time of the whole group, most first; children keep the input order.
 */
export function groupSites(rows: SiteRow[]): SiteGroup[] {
  const hosts = new Set(rows.map((r) => r.host));
  const byRoot = new Map<string, { root?: SiteRow; children: SiteRow[] }>();
  for (const row of rows) {
    const rootHost = groupRoot(row.host, hosts);
    const entry = byRoot.get(rootHost) ?? { children: [] };
    if (rootHost === row.host) entry.root = row;
    else entry.children.push(row);
    byRoot.set(rootHost, entry);
  }

  const groups: (SiteGroup & { sum30d: number; sumTotal: number })[] = [];
  for (const { root, children } of byRoot.values()) {
    // Every root host comes from `rows`, so the root row is always there.
    if (!root) continue;
    const all = [root, ...children];
    groups.push({
      root,
      children,
      otherCategoryCount: children.filter((c) => c.categoryId !== root.categoryId).length,
      sum30d: all.reduce((sum, r) => sum + r.minutes30d, 0),
      sumTotal: all.reduce((sum, r) => sum + r.minutesTotal, 0),
    });
  }
  groups.sort(
    (a, b) =>
      b.sum30d - a.sum30d || b.sumTotal - a.sumTotal || a.root.host.localeCompare(b.root.host),
  );
  return groups.map(({ root, children, otherCategoryCount }) => ({
    root,
    children,
    otherCategoryCount,
  }));
}

/** A group as the search shows it. */
export interface VisibleGroup {
  group: SiteGroup;
  /** The children to list when the group is open */
  children: SiteRow[];
  /** The search matched only children, so the group opens to show them. */
  forceOpen: boolean;
}

/**
 * Applies the search, ignoring case. A group whose website matches keeps all its children; a
 * group where only some children match shows just those, opened.
 */
export function filterGroups(groups: SiteGroup[], search: string): VisibleGroup[] {
  const needle = search.trim().toLowerCase();
  const matches = (row: SiteRow) => row.host.toLowerCase().includes(needle);
  const visible: VisibleGroup[] = [];
  for (const group of groups) {
    if (!needle || matches(group.root)) {
      visible.push({ group, children: group.children, forceOpen: false });
      continue;
    }
    const children = group.children.filter(matches);
    if (children.length > 0) visible.push({ group, children, forceOpen: true });
  }
  return visible;
}
