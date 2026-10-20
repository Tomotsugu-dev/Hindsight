/** One plugin the plugin background window runs, as `plugin_background_list` returns it. */
export type BackgroundPlugin = { id: string; launcherUrl: string };

/**
 * Compares the running iframes with a new plugin list.
 *
 * `running` maps a plugin id to the launcher URL its iframe opened. A plugin whose URL changed
 * (a new version) is in both `remove` and `add`: remove the old iframe first, so the old and new
 * Workers never write the same storage at once.
 */
export function diffFrames(
  running: Map<string, string>,
  next: BackgroundPlugin[],
): { remove: string[]; add: BackgroundPlugin[] } {
  const nextUrls = new Map(next.map((p) => [p.id, p.launcherUrl]));
  const remove = [...running]
    .filter(([id, url]) => nextUrls.get(id) !== url)
    .map(([id]) => id);
  const add = next.filter((p) => running.get(p.id) !== p.launcherUrl);
  return { remove, add };
}
