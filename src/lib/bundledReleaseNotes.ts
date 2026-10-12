/** Release notes for the About page, read from the app bundle.
 *
 * `docs/release-notes/v<version>.md` is the single source of truth: release.yml uses it as the
 * GitHub Release body, and the build embeds the same files with `?raw`. Reading the bundled copies
 * makes the dialog instant and keeps it working offline, with no network request.
 *
 * `eager: false` keeps one lazy chunk per version, so only the versions the user opens are loaded.
 * Every matched file is still built into the app, so versions before 0.8.0 are left out.
 */
const files = import.meta.glob(["../../docs/release-notes/v*.md", "!**/v0.[0-7].*.md"], {
  query: "?raw",
  import: "default",
}) as Record<string, () => Promise<string>>;

/** "../../docs/release-notes/v0.9.3.md" → "0.9.3". */
function versionOf(key: string): string {
  return key.slice(key.lastIndexOf("/v") + 2, -".md".length);
}

/** Sort comparator, newest first. On the same numbers a pre-release ("0.8.6-beta.1") is older. */
function newestFirst(a: string, b: string): number {
  const [coreA, preA = ""] = a.split("-", 2);
  const [coreB, preB = ""] = b.split("-", 2);
  const numsA = coreA.split(".").map(Number);
  const numsB = coreB.split(".").map(Number);
  for (let i = 0; i < Math.max(numsA.length, numsB.length); i++) {
    const diff = (numsB[i] ?? 0) - (numsA[i] ?? 0);
    if (diff !== 0) return diff;
  }
  if (!preA !== !preB) return preA ? 1 : -1;
  return preB.localeCompare(preA);
}

/** Versions that have bundled notes, newest first. */
export const BUNDLED_VERSIONS: readonly string[] = Object.keys(files)
  .map(versionOf)
  .sort(newestFirst);

/** Removes the leading "【0.9.3】" line, for pages that show the version as a heading of their own. */
export function dropVersionLine(text: string, version: string): string {
  const line = `【${version}】`;
  return text.startsWith(line) ? text.slice(line.length).trimStart() : text;
}

/** Returns the bundled notes for `version` ("0.9.3" or "v0.9.3"), or null if there is no such file. */
export async function loadBundledReleaseNotes(version: string): Promise<string | null> {
  const want = version.replace(/^v/i, "");
  const key = Object.keys(files).find((k) => k.endsWith(`/v${want}.md`));
  if (!key) return null;
  return files[key]();
}
