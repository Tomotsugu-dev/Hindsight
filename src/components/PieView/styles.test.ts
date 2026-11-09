import { describe, expect, it } from "vitest";
import DRILL_CSS from "./PieDrillDetail.module.css?raw";
import PIE_CSS from "./PieView.module.css?raw";

/** 取出一个选择器第一次出现时的那段声明。 */
function block(css: string, selector: string): string {
  const start = css.indexOf(`${selector} {`);
  expect(start, selector).toBeGreaterThanOrEqual(0);
  return css.slice(start, css.indexOf("}", start));
}

// Dark mode needs no separate rule.
describe("PieView styles in dark mode", () => {
  it("hovers category rows with the theme's hover color", () => {
    expect(block(PIE_CSS, ".row:hover:not(:disabled)")).toContain("var(--hover-bg)");
  });

  it("needs no dark mode rules for it", () => {
    expect(PIE_CSS).not.toContain("data-mode");
    expect(DRILL_CSS).not.toContain("data-mode");
  });
});
