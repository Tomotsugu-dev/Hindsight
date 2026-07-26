import { describe, expect, it } from "vitest";
import { adjustCategoryColor } from "./categoryColor";

describe("adjustCategoryColor", () => {
  it("亮色主题原样返回", () => {
    expect(adjustCategoryColor("#a78bfa", false)).toBe("#a78bfa");
  });
  it("空值原样返回(不产出 color-mix(空))", () => {
    expect(adjustCategoryColor("", true)).toBe("");
  });
  it("暗色主题掺灰:产出 color-mix 且保留原色与比例", () => {
    const out = adjustCategoryColor("#a78bfa", true);
    expect(out).toContain("color-mix(in oklab");
    expect(out).toContain("#a78bfa 72%");
  });
});
