// The demo has no database, so it keeps the UI state in the browser's localStorage (ADR-0020).
// main-demo.tsx imports this file before any page, so the values are there when a page module
// reads them as it loads.
import { initUiState } from "@app/state/uiState";

const PREFIX = "hindsight.";

/** Returns every `hindsight.` key in localStorage and its value. */
export function readDemoUiState(): Record<string, string> {
  const entries: Record<string, string> = {};
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      const value = key === null ? null : localStorage.getItem(key);
      if (key?.startsWith(PREFIX) && value !== null) entries[key] = value;
    }
  } catch {
    // No localStorage (private mode, tests): start with the defaults
  }
  return entries;
}

/** Saves the value of `key`; `null` deletes the key. */
export function saveDemoUiState(key: string, value: string | null): void {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // Not saved: the change lasts until the page is closed
  }
}

initUiState(readDemoUiState(), saveDemoUiState);
