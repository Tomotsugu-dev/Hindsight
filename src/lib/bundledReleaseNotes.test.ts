import { describe, expect, it } from "vitest";
import { loadBundledReleaseNotes } from "./bundledReleaseNotes";

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
