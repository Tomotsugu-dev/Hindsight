// Loads the main window's UI state before the window renders (ADR-0020). The first time a database
// is opened, the values that older versions kept in localStorage are imported into it. After
// that, localStorage is not read or written.
import { api } from "../api/hindsight";
import { logError } from "../lib/logger";
import { initUiState } from "./uiState";

/** Set in the database by the import. Same key as `MIGRATED_KEY` in src-tauri/src/repo/ui_state.rs. */
export const IMPORTED_KEY = "hindsight.localStorage.imported";

/**
 * The keys that versions up to 0.12.1 kept in localStorage. Keys added later were never in
 * localStorage, so this list does not grow.
 */
export const LEGACY_KEYS: readonly string[] = [
  "hindsight.locale",
  "hindsight.theme",
  "hindsight.theme.custom",
  "hindsight.device.selected",
  "hindsight.plugins.seen",
  "hindsight.stats.view.today",
  "hindsight.stats.view.week",
  "hindsight.stats.view.month",
  "hindsight.stats.pieDepth.today",
  "hindsight.stats.pieDepth.week",
  "hindsight.stats.pieDepth.month",
  "hindsight.stats.barLevel.today",
  "hindsight.stats.barLevel.week",
  "hindsight.stats.barLevel.month",
  "hindsight.stats.cards.today",
  "hindsight.stats.cards.week",
  "hindsight.stats.cards.month",
  "hindsight.stats.mode.week",
  "hindsight.stats.mode.month",
  "hindsight.apps.filter",
  "hindsight.chat.lastConversation",
];

/** Reads the UI state, imports the old values if needed, and keeps them for {@link getUiState}. */
export async function loadUiState(): Promise<void> {
  const save = (key: string, value: string | null) => {
    api.setUiState(key, value).catch((e) => logError("uiState.save", e));
  };
  initUiState(await readValues(), save);
}

async function readValues(): Promise<Record<string, string>> {
  let stored: Record<string, string>;
  try {
    stored = await api.getUiState();
  } catch (e) {
    // The defaults, without the marker, so the next start tries again
    logError("uiState.load", e);
    return {};
  }
  if (IMPORTED_KEY in stored) return stored;
  const legacy = readLocalStorage();
  try {
    return await api.importUiState(legacy);
  } catch (e) {
    // Not imported: use the old values this time, and import at the next start
    logError("uiState.import", e);
    return { ...legacy, ...stored };
  }
}

function readLocalStorage(): Record<string, string> {
  const entries: Record<string, string> = {};
  try {
    for (const key of LEGACY_KEYS) {
      const value = localStorage.getItem(key);
      if (value !== null) entries[key] = value;
    }
  } catch (e) {
    logError("uiState.readLocalStorage", e);
  }
  return entries;
}
