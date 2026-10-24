import { useEffect, useState } from "react";
import { api, type SiteIconPaths } from "../../api/hindsight";
import { logError } from "../../lib/logger";
import { createSiteIconLoader } from "./siteIcons";

// The queue survives page unmounts while an accepted backend batch finishes.
const loadIcons = createSiteIconLoader(api);

export function useSiteIcons(hosts: string[] | null, enabled: boolean) {
  const [icons, setIcons] = useState<SiteIconPaths>({});
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  // Category changes can reload the same websites. Restart only when their order changes.
  const hostKey = hosts?.join("\n") ?? null;

  useEffect(() => {
    if (hostKey === null) return;
    let active = true;
    setFailed(false);
    void loadIcons({
      hosts: hostKey ? hostKey.split("\n") : [],
      enabled,
      isActive: () => active,
      onIcons: setIcons,
    }).catch((error: unknown) => {
      if (!active) return;
      logError("sites.icons", error);
      setFailed(true);
    });
    return () => {
      active = false;
    };
  }, [hostKey, enabled, attempt]);

  return { icons, failed, retry: () => setAttempt((value) => value + 1) };
}
