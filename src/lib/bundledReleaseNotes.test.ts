import { describe, expect, it, vi } from "vitest";
import { loadBundledReleaseNotes, loadReleaseNotes } from "./bundledReleaseNotes";

describe("loadBundledReleaseNotes", () => {
  it("finds the notes by version, with or without a leading v", async () => {
    const a = await loadBundledReleaseNotes("0.9.3");
    const b = await loadBundledReleaseNotes("v0.9.3");
    expect(a).toContain("【0.9.3】");
    expect(b).toBe(a);
  });

  it("returns null for a missing version instead of matching another one", async () => {
    expect(await loadBundledReleaseNotes("0.9")).toBeNull();
    expect(await loadBundledReleaseNotes("999.0.0")).toBeNull();
  });
});

describe("loadReleaseNotes", () => {
  it("uses the bundled notes without going online", async () => {
    const fetchRemote = vi.fn(() => Promise.resolve("remote body"));
    const got = await loadReleaseNotes("0.9.3", fetchRemote);
    expect(got).toContain("【0.9.3】");
    expect(fetchRemote).not.toHaveBeenCalled();
  });

  it("falls back to GitHub only when the bundle has no such version", async () => {
    expect(await loadReleaseNotes("999.0.0", () => Promise.resolve("remote body"))).toBe(
      "remote body",
    );
  });

  it("returns null when the bundle has nothing and the remote fails or is empty", async () => {
    expect(
      await loadReleaseNotes("999.0.0", () => Promise.reject(new Error("offline"))),
    ).toBeNull();
    expect(await loadReleaseNotes("999.0.0", () => Promise.resolve("  "))).toBeNull();
  });
});
