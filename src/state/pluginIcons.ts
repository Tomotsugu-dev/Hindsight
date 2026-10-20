import { useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { onPagesChanged, pluginsApi } from "../api/plugins";
import { logError } from "../lib/logger";

/** The icon URL of each installed plugin that has one, by plugin id. Read again after a plugin is
 * installed, updated, uninstalled, enabled or disabled. */
export function usePluginIcons(): Record<string, string> {
  const [icons, setIcons] = useState<Record<string, string>>({});
  useEffect(() => {
    let alive = true;
    let latest = 0;
    const load = () => {
      const call = ++latest;
      pluginsApi
        .icons()
        .then((paths) => {
          // A slower earlier call must not overwrite a newer list
          if (!alive || call !== latest) return;
          const urls = Object.entries(paths).map(([id, path]) => [id, convertFileSrc(path)]);
          setIcons(Object.fromEntries(urls) as Record<string, string>);
        })
        .catch((e) => logError("plugins.icons", e));
    };
    const unlisten = onPagesChanged(load);
    load();
    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, []);
  return icons;
}
