import { beforeEach, describe, expect, it, vi } from "vitest";

type Entries = Record<string, string>;
const getUiState = vi.fn<() => Promise<Entries>>();
const setUiState = vi.fn<(key: string, value: string | null) => Promise<void>>();
const importUiState = vi.fn<(entries: Entries) => Promise<Entries>>();
vi.mock("../api/hindsight", () => ({
  api: {
    getUiState: () => getUiState(),
    setUiState: (key: string, value: string | null) => setUiState(key, value),
    importUiState: (entries: Entries) => importUiState(entries),
  },
}));

import { IMPORTED_KEY, loadUiState } from "./loadUiState";
import { getUiState as read, setUiState as write } from "./uiState";

/** A localStorage holding `entries`. Returns the list of every call made to it. */
function stubLocalStorage(entries: Entries): string[] {
  const calls: string[] = [];
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => {
      calls.push(`get ${key}`);
      return entries[key] ?? null;
    },
    setItem: (key: string) => void calls.push(`set ${key}`),
    removeItem: (key: string) => void calls.push(`remove ${key}`),
  });
  return calls;
}

describe("loadUiState", () => {
  beforeEach(() => {
    getUiState.mockReset();
    setUiState.mockReset().mockResolvedValue(undefined);
    importUiState.mockReset();
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  it("copies the known localStorage keys when the marker is missing", async () => {
    stubLocalStorage({ "hindsight.locale": "ja", "hindsight.theme": "dark", "other.key": "x" });
    getUiState.mockResolvedValue({});
    importUiState.mockImplementation((entries) =>
      Promise.resolve({ ...entries, [IMPORTED_KEY]: "1" }),
    );

    await loadUiState();

    expect(importUiState).toHaveBeenCalledWith({
      "hindsight.locale": "ja",
      "hindsight.theme": "dark",
    });
    expect(read("hindsight.locale")).toBe("ja");
  });

  it("does not touch localStorage once the marker is set", async () => {
    const calls = stubLocalStorage({ "hindsight.locale": "en" });
    getUiState.mockResolvedValue({ [IMPORTED_KEY]: "1", "hindsight.locale": "ja" });

    await loadUiState();
    expect(read("hindsight.locale")).toBe("ja");
    write("hindsight.locale", "zh-CN");

    expect(calls).toEqual([]);
    expect(importUiState).not.toHaveBeenCalled();
    expect(setUiState).toHaveBeenCalledWith("hindsight.locale", "zh-CN");
    expect(read("hindsight.locale")).toBe("zh-CN");
  });

  it("starts with defaults when loading fails", async () => {
    stubLocalStorage({ "hindsight.locale": "ja" });
    getUiState.mockRejectedValue(new Error("database is locked"));

    await loadUiState();

    expect(read("hindsight.locale")).toBeNull();
    expect(importUiState).not.toHaveBeenCalled();
  });

  it("uses the old values for this start when the import fails", async () => {
    stubLocalStorage({ "hindsight.locale": "ja" });
    getUiState.mockResolvedValue({});
    importUiState.mockRejectedValue(new Error("database is locked"));

    await loadUiState();

    expect(read("hindsight.locale")).toBe("ja");
  });
});
