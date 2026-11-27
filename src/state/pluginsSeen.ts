import { useSyncExternalStore } from "react";
import { getUiState, setUiState } from "./uiState";

// Whether the user has opened Settings → Plugins. Until then, 设置 in the sidebar and the
// Plugins tab show a 「新」 badge, so users who updated find the new plugin market.
const KEY = "hindsight.plugins.seen";

let seen = getUiState(KEY) === "1";
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
  setUiState(KEY, "1");
  for (const listener of listeners) listener();
}
