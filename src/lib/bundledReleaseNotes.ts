/** Release notes of the running version for the About page, read from the app bundle.
 *
 * `docs/release-notes/v<version>.md` is the single source of truth: release.yml uses it as the
 * GitHub Release body, and the build embeds the same file with `?raw`. Reading the bundled copy
 * makes the dialog instant and keeps it working offline, with no network request.
 *
 * `eager: false` keeps one lazy chunk per version, so only the running version's notes are
 * loaded, and only when the dialog opens.
 */
const files = import.meta.glob("../../docs/release-notes/v*.md", {
  query: "?raw",
  import: "default",
}) as Record<string, () => Promise<string>>;

/** Returns the bundled notes for `version` ("0.9.3" or "v0.9.3"), or null if there is no such file. */
export async function loadBundledReleaseNotes(version: string): Promise<string | null> {
  const want = version.replace(/^v/i, "");
  const key = Object.keys(files).find((k) => k.endsWith(`/v${want}.md`));
  if (!key) return null;
  return files[key]();
}
