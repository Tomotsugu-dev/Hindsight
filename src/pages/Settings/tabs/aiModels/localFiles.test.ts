import { describe, expect, it } from "vitest";
import type { ModelEntry, RecommendedModel } from "../../../../api/hindsight";
import { groupLocalFiles, isRecommendedInstalled, mmprojSaveAs } from "./localFiles";

function file(filename: string, isMmproj = false): ModelEntry {
  return { filename, path: `/models/${filename}`, sizeBytes: 1, isMmproj, supportsTools: null };
}

function rec(mainFile: string, mmprojFile: string | null = null): RecommendedModel {
  return {
    displayName: mainFile,
    repo: "org/repo",
    mainFile,
    mainBytes: 1,
    mmprojFile,
    mmprojBytes: 0,
    logoUrl: null,
    vision: false,
    brand: "",
    caps: [],
    released: "",
  };
}

describe("mmprojSaveAs", () => {
  it("prefixes the mmproj with the main file's stem", () => {
    expect(mmprojSaveAs(rec("qwen-4b.gguf", "mmproj-F16.gguf"))).toBe("qwen-4b__mmproj-F16.gguf");
  });

  it("is empty without an mmproj", () => {
    expect(mmprojSaveAs(rec("qwen-4b.gguf"))).toBe("");
  });
});

describe("isRecommendedInstalled", () => {
  it("only needs the main file", () => {
    expect(isRecommendedInstalled(rec("a.gguf", "m.gguf"), [file("a.gguf")])).toBe(true);
    expect(isRecommendedInstalled(rec("a.gguf"), [file("b.gguf")])).toBe(false);
  });
});

describe("groupLocalFiles", () => {
  it("lists downloaded recommended models in the list's order, with their saved mmproj", () => {
    const recs = [rec("b.gguf"), rec("a.gguf", "mmproj-F16.gguf")];
    const items = groupLocalFiles(
      [file("a.gguf"), file("a__mmproj-F16.gguf", true), file("b.gguf")],
      recs,
    );
    expect(items.map((i) => i.kind)).toEqual(["recommended", "recommended"]);
    expect(items[0].kind === "recommended" && items[0].rec.mainFile).toBe("b.gguf");
    expect(items[1].kind === "recommended" && items[1].mmproj?.filename).toBe("a__mmproj-F16.gguf");
  });

  it("pairs an imported file with the mmproj whose name matches", () => {
    const items = groupLocalFiles(
      [
        file("gemma-4-12B-it-Q4_K_M.gguf"),
        file("mmproj-gemma-4-12B-it-BF16.gguf", true),
        file("mmproj-llava-7b.gguf", true),
      ],
      [],
    );
    expect(items[0]).toMatchObject({
      kind: "file",
      main: { filename: "gemma-4-12B-it-Q4_K_M.gguf" },
      mmproj: { filename: "mmproj-gemma-4-12B-it-BF16.gguf" },
    });
    // 配不上的 mmproj 单独列，排在最后
    expect(items[1]).toMatchObject({
      kind: "orphanMmproj",
      file: { filename: "mmproj-llava-7b.gguf" },
    });
  });

  it("does not pair on one matching segment", () => {
    const items = groupLocalFiles(
      [file("gemma-31b.gguf"), file("mmproj-gemma-12b.gguf", true), file("other.gguf")],
      [],
    );
    expect(items.filter((i) => i.kind === "orphanMmproj")).toHaveLength(1);
  });

  it("pairs the last file and the last mmproj when names do not match", () => {
    // LM Studio 给 mmproj 起通用名，前缀配不上；只剩一对时直接配
    const items = groupLocalFiles(
      [file("gemma-3-27b.gguf"), file("mmproj-model-f16.gguf", true)],
      [],
    );
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ kind: "file", mmproj: { filename: "mmproj-model-f16.gguf" } });
  });
});
