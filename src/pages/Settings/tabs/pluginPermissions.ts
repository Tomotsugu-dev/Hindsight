// Plugin permissions as the install dialog shows them. The list and risk levels are in
// docs/design/plugins/en/permissions.md.

/**
 * Permissions marked in the install dialog: they show what the user did in detail, or change
 * Hindsight's data.
 */
export const HIGH_RISK = new Set([
  "focus.title",
  "categories.app.write",
  "categories.site.read",
  "categories.site.write",
]);

/** stats.read must keep at least one range under it, the same rule as the manifest check in the
 * backend */
const NEEDS_A_RANGE = "stats.read";

/** The i18n key part for a permission: i18next reads "." as nesting, so it becomes "_". */
export function permissionKey(permission: string): string {
  return permission.replace(/\./g, "_");
}

/** One permission in the tree, with the permissions that depend on it under it. */
export interface PermissionNode {
  name: string;
  children: PermissionNode[];
}

/** Puts the asked permissions in a tree: each one sits under the one it depends on. Each level
 * keeps the manifest's order. */
export function permissionTree(
  asked: string[],
  dependsOn: Record<string, string>,
): PermissionNode[] {
  const node = (name: string): PermissionNode => ({
    name,
    children: asked.filter((p) => dependsOn[p] === name).map(node),
  });
  return asked.filter((p) => !asked.includes(dependsOn[p])).map(node);
}

/** Returns every permission under a permission, at any depth. */
export function permissionsUnder(node: PermissionNode): string[] {
  return node.children.flatMap((c) => [c.name, ...permissionsUnder(c)]);
}

/** Checks or unchecks a permission, and returns the new checked list in the manifest's order.
 *
 * Checking one also checks the one it depends on. Checking stats.read with no range checked
 * checks every range. Unchecking one also unchecks the ones that depend on it. Unchecking the
 * last range also unchecks stats.read. */
export function setChecked(
  checked: string[],
  permission: string,
  on: boolean,
  asked: string[],
  dependsOn: Record<string, string>,
): string[] {
  const next = new Set(checked);
  if (on) {
    for (let p: string | undefined = permission; p; p = dependsOn[p]) next.add(p);
    const ranges = asked.filter((p) => dependsOn[p] === NEEDS_A_RANGE);
    if (next.has(NEEDS_A_RANGE) && !ranges.some((r) => next.has(r))) {
      ranges.forEach((r) => next.add(r));
    }
  } else {
    next.delete(permission);
  }
  return fit(next, asked, dependsOn);
}

/** Returns what is checked at first when installing over an installed version. What the user
 * approved stays checked, what the user said no to stays unchecked, and what the new version
 * newly asks for is checked. */
export function keptChoices(
  asked: string[],
  oldAsked: string[],
  oldApproved: string[],
  dependsOn: Record<string, string>,
): string[] {
  const kept = asked.filter((p) => oldApproved.includes(p) || !oldAsked.includes(p));
  return fit(new Set(kept), asked, dependsOn);
}

/** Drops each permission whose dependency is not checked, and stats.read with no range. */
function fit(checked: Set<string>, asked: string[], dependsOn: Record<string, string>): string[] {
  const holds = (p: string): boolean =>
    checked.has(p) && (dependsOn[p] === undefined || holds(dependsOn[p]));
  const kept = asked.filter(holds);
  const ranges = asked.filter((p) => dependsOn[p] === NEEDS_A_RANGE);
  if (!ranges.some((r) => kept.includes(r))) {
    return kept.filter((p) => p !== NEEDS_A_RANGE);
  }
  return kept;
}
