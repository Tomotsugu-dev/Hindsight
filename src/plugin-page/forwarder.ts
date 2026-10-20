// The forwarder for plugin pages: passes hindsight.call() from a plugin page in an iframe to
// Rust, and the result back. The page in the main window and the page's own window both use it.
// See docs/design/plugins/en/runtime.md.
import { invoke } from "@tauri-apps/api/core";
import { DEVICE_SELECTED_KEY } from "../state/deviceFilterKey";

/** A `hindsight.call()` from a plugin, see `_page.js` and `_worker.js` in file_reader.rs. */
export type PluginRequest = { id: number; method: string; params?: unknown };

export function isRequest(data: unknown): data is PluginRequest {
  if (typeof data !== "object" || data === null) return false;
  const { id, method } = data as Record<string, unknown>;
  return typeof id === "number" && typeof method === "string";
}

/** The origin of a plugin page URL, such as `hindsight-plugin://focus.localhost`. Cut by hand:
 * `new URL(...).origin` is "null" for a scheme the URL standard does not know. */
export function originOf(url: string): string {
  const start = url.indexOf("//") + 2;
  const end = url.indexOf("/", start);
  return end === -1 ? url : url.slice(0, end);
}

/** Forwards the calls of plugin `pluginId`, whose page `url` is in `frame`. Returns a function
 * that stops forwarding. */
export function forwardCalls(frame: HTMLIFrameElement, pluginId: string, url: string): () => void {
  const origin = originOf(url);
  const onMessage = (event: MessageEvent) => {
    // A page can navigate its own iframe to another site. Only the plugin's own origin may call,
    // and replies go only to that origin
    if (event.source !== frame.contentWindow || event.origin !== origin) return;
    if (!isRequest(event.data)) return;
    const { id, method, params } = event.data;
    const reply = (message: object) => frame.contentWindow?.postMessage(message, origin);
    // The device selected in the main window. Same origin as the main window, so this window
    // reads the same value. It is not in params, so the plugin cannot change it
    const device = localStorage.getItem(DEVICE_SELECTED_KEY);
    invoke("plugin_call", { pluginId, method, params: params ?? null, device }).then(
      (result) => reply({ id, result }),
      (error) => reply({ id, error: String(error) }),
    );
  };
  window.addEventListener("message", onMessage);
  return () => window.removeEventListener("message", onMessage);
}

/** `sandbox` for the page's iframe: scripts and its own storage only. No forms, popups or
 * navigating the outer page. */
export const PAGE_SANDBOX = "allow-scripts allow-same-origin";
