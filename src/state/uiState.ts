// The main window's UI state, such as the language and the theme (ADR-0020). It is read from the
// database once before the window renders and kept here, so reads are synchronous. A change is
// kept here at once and saved without waiting.
//
// This file imports nothing: lib/theme.ts uses it, and the themes site imports lib/theme.ts
// without the Tauri API.

/** Saves one change; `null` deletes the key. */
export type UiStateSaver = (key: string, value: string | null) => void;

let values = new Map<string, string>();
let save: UiStateSaver = () => {};

/** Replaces every value, and sets how later changes are saved. */
export function initUiState(entries: Record<string, string>, saver: UiStateSaver): void {
  values = new Map(Object.entries(entries));
  save = saver;
}

/** Returns the value of `key`, or null if it has none. */
export function getUiState(key: string): string | null {
  return values.get(key) ?? null;
}

/** Keeps the value of `key` and saves it; `null` deletes the key. An unchanged value is not saved. */
export function setUiState(key: string, value: string | null): void {
  if ((values.get(key) ?? null) === value) return;
  if (value === null) values.delete(key);
  else values.set(key, value);
  save(key, value);
}
