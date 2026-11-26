import { useSyncExternalStore } from "react";
import { api, type SyncStatus } from "../api/hindsight";

// Whether background sync is paused on this device. The sidebar cloud and the Cloud Sync page both
// show it and both change it, so they share this one copy: a click on either updates both at once.
let paused = false;
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function update(next: boolean) {
  if (paused === next) return;
  paused = next;
  for (const listener of listeners) listener();
}

export function useSyncPaused(): boolean {
  return useSyncExternalStore(subscribe, () => paused);
}

/** Fetches the sync status and takes the pause from it. */
export async function loadSyncStatus(): Promise<SyncStatus> {
  const status = await api.syncStatus();
  update(status.paused);
  return status;
}

/** Pauses or resumes background sync on this device. */
export async function setSyncPaused(next: boolean): Promise<void> {
  await api.setSyncPaused(next);
  update(next);
}
