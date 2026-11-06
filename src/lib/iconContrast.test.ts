import { describe, expect, it } from "vitest";
import { contrastRatio, isDarkIcon, relativeLuminance } from "./iconContrast";

/** RGBA 像素：每一项是一种颜色和它占几个像素。 */
function pixels(...parts: [r: number, g: number, b: number, a: number, count: number][]) {
  const out: number[] = [];
  for (const [r, g, b, a, count] of parts) {
    for (let i = 0; i < count; i++) out.push(r, g, b, a);
  }
  return new Uint8ClampedArray(out);
}

describe("isDarkIcon", () => {
  it("treats a black icon as dark", () => {
    expect(isDarkIcon(pixels([0, 0, 0, 255, 100]))).toBe(true);
  });

  it("does not treat a red icon as dark", () => {
    // Quora 的红色：对比度 2.95，不到 3，但看得清
    expect(isDarkIcon(pixels([0xb9, 0x2b, 0x27, 255, 100]))).toBe(false);
  });

  it("treats a navy icon as dark", () => {
    // PowerShell 的深蓝：对比度 1.19
    expect(isDarkIcon(pixels([0x01, 0x24, 0x56, 255, 100]))).toBe(true);
  });

  it("treats an icon with a small light glyph as dark", () => {
    // 黑方块上的白色标志，像 X：白色只占 17%
    expect(isDarkIcon(pixels([255, 255, 255, 255, 17], [0, 0, 0, 255, 83]))).toBe(true);
  });

  it("draws the line at 35% visible pixels", () => {
    expect(isDarkIcon(pixels([255, 255, 255, 255, 34], [0, 0, 0, 255, 66]))).toBe(true);
    expect(isDarkIcon(pixels([255, 255, 255, 255, 35], [0, 0, 0, 255, 65]))).toBe(false);
  });

  it("ignores pixels that are mostly transparent", () => {
    // 一半透明的白边不算图标本身，剩下的黑色部分还是看不见
    expect(isDarkIcon(pixels([0, 0, 0, 255, 20], [255, 255, 255, 128, 80]))).toBe(true);
  });

  it("does not treat a fully transparent icon as dark", () => {
    expect(isDarkIcon(pixels([0, 0, 0, 0, 100]))).toBe(false);
  });
});

describe("relativeLuminance", () => {
  it("weighs green most and blue least", () => {
    expect(relativeLuminance(0, 255, 0)).toBeCloseTo(0.7152, 4);
    expect(relativeLuminance(255, 0, 0)).toBeCloseTo(0.2126, 4);
    expect(relativeLuminance(0, 0, 255)).toBeCloseTo(0.0722, 4);
  });

  it("gives middle gray about a fifth of the light of white", () => {
    expect(relativeLuminance(128, 128, 128)).toBeCloseTo(0.2159, 4);
  });
});

describe("contrastRatio", () => {
  it("gives the same contrast in either order", () => {
    const black = relativeLuminance(0, 0, 0);
    const white = relativeLuminance(255, 255, 255);
    expect(contrastRatio(black, white)).toBeCloseTo(21);
    expect(contrastRatio(white, black)).toBeCloseTo(21);
  });

  it("gives 1 for the same color", () => {
    const gray = relativeLuminance(80, 80, 80);
    expect(contrastRatio(gray, gray)).toBe(1);
  });
});
