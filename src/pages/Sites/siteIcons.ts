import type { SiteIconDownloadRound, SiteIconPaths } from "../../api/hindsight";

interface SiteIconBackend {
  getSiteIcons: () => Promise<SiteIconPaths>;
  downloadSiteIcons: (hosts: string[]) => Promise<SiteIconDownloadRound>;
}

interface SiteIconRequest {
  hosts: string[];
  enabled: boolean;
  isActive: () => boolean;
  onIcons: (icons: SiteIconPaths) => void;
}

/**
 * Loads cached icons even when downloads are off, then downloads successive batches if enabled.
 * Requests share a queue so reopening the page cannot start overlapping download batches.
 * Leaving the page or disabling downloads lets the current call finish and stops the next one.
 */
export function createSiteIconLoader(backend: SiteIconBackend) {
  let queue = Promise.resolve();

  return (request: SiteIconRequest): Promise<void> => {
    const task = queue.then(async () => {
      if (!request.isActive()) return;
      let icons = await backend.getSiteIcons();
      if (!request.isActive()) return;
      request.onIcons(icons);

      let previousRemaining = Infinity;
      while (request.enabled && request.hosts.length > 0 && request.isActive()) {
        const round = await backend.downloadSiteIcons(request.hosts);
        if (!request.isActive()) return;
        icons = { ...icons, ...round.icons };
        request.onIcons(icons);
        if (round.remaining === 0) return;
        // A cache write can fail. Avoid requesting the same failed batch indefinitely.
        if (round.remaining >= previousRemaining) {
          throw new Error("Website icon downloads made no progress");
        }
        previousRemaining = round.remaining;
      }
    });

    // An unsuccessful request must not prevent the next page visit from loading icons.
    queue = task.catch(() => {});
    return task;
  };
}
