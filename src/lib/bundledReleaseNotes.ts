/** Release notes of the running version for the About page.
 *
 * `docs/release-notes/v<version>.md` is the single source of truth: release.yml uses it as the
 * GitHub Release body, and the build embeds the same file with `?raw`. Reading the bundled copy
 * first makes the dialog instant and keeps it working offline or when GitHub is unreachable;
 * GitHub is only a fallback for a version whose file is missing from the bundle.
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

/** Returns the raw notes for `version`: the bundled copy if present, else `fetchRemote` (GitHub).
 * A rejected or empty remote answer counts as "none"; the caller shows its "no release notes"
 * text when this returns null. */
export async function loadReleaseNotes(
  version: string,
  fetchRemote: (version: string) => Promise<string>,
): Promise<string | null> {
  const bundled = await loadBundledReleaseNotes(version);
  if (bundled) return bundled;
  try {
    const remote = await fetchRemote(version);
    if (remote.trim()) return remote;
  } catch {
    // Being offline is not an error here.
  }
  return null;
}
