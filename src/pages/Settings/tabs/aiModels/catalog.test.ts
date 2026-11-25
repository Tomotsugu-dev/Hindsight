import { describe, expect, it } from "vitest";
import type { RecommendedModel } from "../../../../api/hindsight";
import {
  COLLAPSED_COUNT,
  SMALL_BYTES,
  catalogList,
  matchesFilter,
  visibleCatalog,
} from "./catalog";

function rec(name: string, opts: Partial<RecommendedModel> = {}): RecommendedModel {
  return {
    displayName: name,
    repo: "org/repo",
    mainFile: `${name}.gguf`,
    mainBytes: 1,
    mmprojFile: null,
    mmprojBytes: 0,
    logoUrl: null,
    vision: false,
    brand: "",
    caps: [],
    released: "",
    ...opts,
  };
}

const none = () => false;

describe("matchesFilter", () => {
  it("sorts models into the chips by their caps and size", () => {
    const vision = rec("v", { caps: ["VISION", "TEXT"] });
    const r1 = rec("r", { caps: ["R1"] });
    const big = rec("b", { mainBytes: SMALL_BYTES });
    expect(matchesFilter(vision, "vision")).toBe(true);
    expect(matchesFilter(r1, "vision")).toBe(false);
    expect(matchesFilter(r1, "reasoning")).toBe(true);
    expect(matchesFilter(rec("q", { caps: ["REASONING"] }), "reasoning")).toBe(true);
    // 主文件加 mmproj 一起算；正好 5 GB 不算小
    expect(matchesFilter(big, "small")).toBe(false);
    expect(matchesFilter(rec("s", { mainBytes: SMALL_BYTES - 2, mmprojBytes: 1 }), "small")).toBe(
      true,
    );
  });
});

describe("catalogList", () => {
  const list = [
    rec("b", { brand: "Qwen", mainBytes: 3, released: "2025-01-01" }),
    rec("a", { brand: "Google", mainBytes: 1, released: "" }),
    rec("c", { brand: "Qwen", mainBytes: 2, released: "2026-01-01" }),
  ];

  it("leaves out downloaded models", () => {
    const names = catalogList(list, (r) => r.displayName === "a", "all", "all", "default");
    expect(names.map((r) => r.displayName)).toEqual(["b", "c"]);
  });

  it("keeps the list's order by default and filters by brand", () => {
    expect(catalogList(list, none, "all", "Qwen", "default").map((r) => r.displayName)).toEqual([
      "b",
      "c",
    ]);
  });

  it("sorts by size, name and release date", () => {
    const names = (sort: Parameters<typeof catalogList>[4]) =>
      catalogList(list, none, "all", "all", sort).map((r) => r.displayName);
    expect(names("size-asc")).toEqual(["a", "c", "b"]);
    expect(names("size-desc")).toEqual(["b", "c", "a"]);
    expect(names("name")).toEqual(["a", "b", "c"]);
    // 没有日期的排最后
    expect(names("released")).toEqual(["c", "b", "a"]);
  });
});

describe("visibleCatalog", () => {
  const many = Array.from({ length: COLLAPSED_COUNT + 3 }, (_, i) => rec(`m${i}`));

  it("shows the first models until expanded", () => {
    expect(visibleCatalog(many, false, none)).toHaveLength(COLLAPSED_COUNT);
    expect(visibleCatalog(many, true, none)).toHaveLength(many.length);
  });

  it("keeps a later model that is downloading", () => {
    const last = many[many.length - 1];
    const shown = visibleCatalog(many, false, (r) => r === last);
    expect(shown).toHaveLength(COLLAPSED_COUNT + 1);
    expect(shown[shown.length - 1]).toBe(last);
  });
});
