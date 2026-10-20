// The Hindsight page in a plugin page's own window: shows the plugin page in an iframe and
// forwards its calls. See docs/design/plugins/en/runtime.md.
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { onPagesChanged, pluginsApi, type PluginPage } from "../api/plugins";
import { forwardCalls, PAGE_SANDBOX } from "../plugin-page/forwarder";

// The label is "plugin-window-<id>", see plugins/pages.rs
const pluginId = getCurrentWebviewWindow().label.replace(/^plugin-window-/, "");

let shown: { url: string; frame: HTMLIFrameElement; stop: () => void } | null = null;

function show(pages: PluginPage[]) {
  const page = pages.find((p) => p.id === pluginId);
  // No page: Rust closes this window. Same URL: same version, nothing to reload
  if (!page || page.url === shown?.url) return;
  shown?.stop();
  shown?.frame.remove();
  const frame = document.createElement("iframe");
  // Before src: sandbox flags apply when the frame loads
  frame.setAttribute("sandbox", PAGE_SANDBOX);
  frame.src = page.url;
  document.body.append(frame);
  shown = { url: page.url, frame, stop: forwardCalls(frame, pluginId, page.url) };
}

async function start() {
  let changed = false;
  // Listen before asking for the list, so a change in between is not missed
  await onPagesChanged((pages) => {
    changed = true;
    show(pages);
  });
  const pages = await pluginsApi.pages();
  // A change that came while waiting has the newer list
  if (!changed) show(pages);
}

start().catch((e) => console.error("plugin window:", e));
