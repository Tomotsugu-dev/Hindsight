/**
 * Turns a process name into the name shown to the user. The original value is
 * still the key and the backend ID.
 *
 * - Removes the program suffix: `code.exe` → `code`, `Safari.app` → `Safari`
 * - Keeps only the last part of a Linux app ID: `org.gnome.Nautilus` → `Nautilus`
 */
export function displayAppName(name: string): string {
  if (!name) return name;
  const stripped = name.replace(/\.(exe|lnk|app)$/i, "");
  // Only IDs with three or more parts that start with a common top-level
  // domain, so a name like `my.exe.tool` is not cut
  const appId = /^(?:org|com|net|io|dev|app)\.(?:[\w-]+\.)+([\w-]+)$/.exec(stripped);
  return appId ? appId[1] : stripped;
}
