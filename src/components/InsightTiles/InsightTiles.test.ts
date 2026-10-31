import { describe, expect, it } from "vitest";
import CSS from "./InsightTiles.module.css?raw";

/** 取出一个选择器第一次出现时的那段声明。 */
function block(selector: string): string {
  const start = CSS.indexOf(`${selector} {`);
  expect(start, selector).toBeGreaterThanOrEqual(0);
  return CSS.slice(start, CSS.indexOf("}", start));
}

describe("InsightTiles styles", () => {
  // 选中大类时卡片换底色：底色写死成浅色，深色模式下浅色的文字就看不清
  it("tints picked-category tiles over the theme's card color", () => {
    const drilled = block(".tiles[data-drill] .tile");
    expect(drilled).toContain("var(--surface-card-elevated)");
    expect(drilled).not.toMatch(/#[0-9a-f]{3,8}\b/i);
  });
});
