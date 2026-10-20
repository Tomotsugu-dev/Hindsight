// Plugin permissions as the install dialog shows them. The list and risk levels are in
// docs/design/plugins/en/permissions.md.

/** Permissions that show what the user did in detail, marked in the install dialog. */
export const HIGH_RISK = new Set(["focus.title"]);

/** The i18n key part for a permission: i18next reads "." as nesting, so it becomes "_". */
export function permissionKey(permission: string): string {
  return permission.replace(/\./g, "_");
}
