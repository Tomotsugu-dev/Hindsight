// The plugin background window: one iframe per plugin, and each iframe opens that plugin's
// launcher. See docs/design/plugins/en/runtime.md.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { DEVICE_SELECTED_KEY } from "../state/deviceFilterKey";
import { isRequest } from "../plugin-page/forwarder";
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

// The plugin is the one whose iframe sent the message. The browser sets event.source, so a
// plugin cannot claim to be another plugin
window.addEventListener("message", (event) => {
  const entry = [...frames].find(([, f]) => f.frame.contentWindow === event.source);
  if (!entry) return;
  const [pluginId, { frame }] = entry;
  // The launcher stopped the plugin's Worker: it did not answer two pings in a row
  const data: unknown = event.data;
  if (typeof data === "object" && data !== null && "stopped" in data && data.stopped === true) {
    invoke("plugin_stopped", { pluginId }).catch((e) => console.error("plugin_stopped:", e));
    return;
  }
  if (!isRequest(event.data)) return;
  const { id, method, params } = event.data;
  // "*": the iframe always holds Hindsight's launcher, and plugin code cannot navigate it.
  // If the plugin was turned off in the meantime, contentWindow is null and the reply is dropped
  const reply = (message: object) => frame.contentWindow?.postMessage(message, "*");
  // The device selected in the main window, read on every call, so a change there applies to
  // the next call. It is not in params, so the plugin cannot change it
  const device = localStorage.getItem(DEVICE_SELECTED_KEY);
  invoke("plugin_call", { pluginId, method, params: params ?? null, device }).then(
    (result) => reply({ id, result }),
    (error) => reply({ id, error: String(error) }),
  );
});

/** An event from Rust for one plugin, such as `focus.changed`. See plugins/events/. */
type PluginEvent = { toPluginId: string; eventName: string; data: unknown };

async function start() {
  // The launcher passes it on to the Worker, where hindsight.on() callbacks get it. An event that
  // comes before the plugin's iframe is open is dropped: a focus event is out of date by the next one
  await listen<PluginEvent>("plugin-event", ({ payload }) => {
    const { toPluginId, eventName, data } = payload;
    frames.get(toPluginId)?.frame.contentWindow?.postMessage({ eventName, data }, "*");
  });
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
