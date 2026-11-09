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

/** The i18n key part for a permission: i18next reads "." as nesting, so it becomes "_". */
export function permissionKey(permission: string): string {
  return permission.replace(/\./g, "_");
}
