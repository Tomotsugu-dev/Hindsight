// The plugin background window: one iframe per plugin, and each iframe opens that plugin's
// launcher. See docs/design/plugins/en/runtime.md.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { type BackgroundPlugin, diffFrames } from "./frames";

const frames = new Map<string, { url: string; frame: HTMLIFrameElement }>();

function show(plugins: BackgroundPlugin[]) {
  const running = new Map([...frames].map(([id, f]) => [id, f.url]));
  const { remove, add } = diffFrames(running, plugins);
  for (const id of remove) {
    frames.get(id)?.frame.remove();
    frames.delete(id);
  }
  for (const plugin of add) {
    const frame = document.createElement("iframe");
    frame.src = plugin.launcherUrl;
    document.body.append(frame);
    frames.set(plugin.id, { url: plugin.launcherUrl, frame });
  }
}

/** A `hindsight.call()` from a plugin's Worker, passed on by its launcher. */
type PluginRequest = { id: number; method: string; params?: unknown };

function isRequest(data: unknown): data is PluginRequest {
  if (typeof data !== "object" || data === null) return false;
  const { id, method } = data as Record<string, unknown>;
  return typeof id === "number" && typeof method === "string";
}

// The plugin is the one whose iframe sent the message. The browser sets event.source, so a
// plugin cannot claim to be another plugin
window.addEventListener("message", (event) => {
  const entry = [...frames].find(([, f]) => f.frame.contentWindow === event.source);
  if (!entry || !isRequest(event.data)) return;
  const [pluginId, { frame }] = entry;
  const { id, method, params } = event.data;
  // "*": the iframe always holds Hindsight's launcher, and plugin code cannot navigate it.
  // If the plugin was turned off in the meantime, contentWindow is null and the reply is dropped
  const reply = (message: object) => frame.contentWindow?.postMessage(message, "*");
  invoke("plugin_call", { pluginId, method, params: params ?? null }).then(
    (result) => reply({ id, result }),
    (error) => reply({ id, error: String(error) }),
  );
});

async function start() {
  let changed = false;
  // Listen before asking for the list, so a change in between is not missed
  await listen<BackgroundPlugin[]>("plugins-changed", (event) => {
    changed = true;
    show(event.payload);
  });
  const plugins = await invoke<BackgroundPlugin[]>("plugin_background_list");
  // An event that came while waiting has the newer list
  if (!changed) show(plugins);
}

start().catch((e) => console.error("plugin background window:", e));
