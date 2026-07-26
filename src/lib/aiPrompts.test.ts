import { describe, expect, it } from "vitest";
import { DEFAULT_SYSTEM_PROMPTS, overrideKey } from "./aiPrompts";

describe("aiPrompts", () => {
  it("五语内置 prompt 全部非空且已去尾空白", () => {
    for (const lang of ["zh", "tw", "en", "ja", "pt"] as const) {
      const text = DEFAULT_SYSTEM_PROMPTS[lang];
      expect(text.length).toBeGreaterThan(100);
      expect(text).toBe(text.trimEnd());
    }
  });
  it("overrideKey 五语映射到各自覆盖字段", () => {
    expect(overrideKey("zh")).toBe("systemZh");
    expect(overrideKey("tw")).toBe("systemTw");
    expect(overrideKey("en")).toBe("systemEn");
    expect(overrideKey("ja")).toBe("systemJa");
    expect(overrideKey("pt")).toBe("systemPt");
  });
});
