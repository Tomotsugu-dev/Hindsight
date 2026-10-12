import { describe, expect, it } from "vitest";
import { BUNDLED_VERSIONS, dropVersionLine, loadBundledReleaseNotes } from "./bundledReleaseNotes";

describe("BUNDLED_VERSIONS", () => {
  it("lists versions newest first, comparing numbers rather than text", () => {
    const order = ["0.9.3", "0.9.2", "0.8.10", "0.8.9", "0.8.6", "0.8.6-beta.1", "0.8.0"].map(
      (v) => BUNDLED_VERSIONS.indexOf(v),
    );
    expect(order).not.toContain(-1);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
  });

  it("stops at 0.8.0", () => {
    expect(BUNDLED_VERSIONS[BUNDLED_VERSIONS.length - 1]).toBe("0.8.0");
  });
});

describe("dropVersionLine", () => {
  it("removes the version line and the blank line after it", () => {
    expect(dropVersionLine("【0.9.3】\n\n**修改**\n- a", "0.9.3")).toBe("**修改**\n- a");
  });

  it("keeps a first line that is another version", () => {
    expect(dropVersionLine("【0.9.3】\n- a", "0.9.2")).toBe("【0.9.3】\n- a");
  });
});

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
