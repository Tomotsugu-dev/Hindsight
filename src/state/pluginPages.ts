import { useEffect, useState } from "react";
import { onPagesChanged, pluginsApi, type PluginPage } from "../api/plugins";
import { logError } from "../lib/logger";

/** The enabled plugins that have a page, kept up to date when plugins change. `null` until the
 * first list arrives. */
export function usePluginPages(): PluginPage[] | null {
  const [pages, setPages] = useState<PluginPage[] | null>(null);
  useEffect(() => {
    let alive = true;
    let changed = false;
    const unlisten = onPagesChanged((next) => {
      changed = true;
      if (alive) setPages(next);
    });
    // Listen first, so a change in between is not missed; a change that came while waiting has
    // the newer list
    void unlisten
      .then(() => pluginsApi.pages())
      .then((first) => {
        if (alive && !changed) setPages(first);
      })
      .catch((e) => logError("plugins.pages", e));
    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, []);
  return pages;
}
