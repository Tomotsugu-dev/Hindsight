import { useSyncExternalStore } from "react";

// Whether the user has opened Settings → Plugins. Until then, 设置 in the sidebar and the
// Plugins tab show a 「新」 badge, so users who updated find the new plugin market.
const KEY = "hindsight.plugins.seen";

function read(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return true;
  }
}

let seen = read();
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePluginsSeen(): boolean {
  return useSyncExternalStore(subscribe, () => seen);
}

export function markPluginsSeen() {
  if (seen) return;
  seen = true;
  try {
    localStorage.setItem(KEY, "1");
  } catch {
    // Not saved: the badge comes back next time, nothing else breaks
  }
  for (const listener of listeners) listener();
}
