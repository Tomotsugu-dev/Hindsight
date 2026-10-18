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
