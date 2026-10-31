import { describe, expect, it } from "vitest";
import DRILL_CSS from "./PieDrillDetail.module.css?raw";
import PIE_CSS from "./PieView.module.css?raw";

/** 取出一个选择器第一次出现时的那段声明。 */
function block(css: string, selector: string): string {
  const start = css.indexOf(`${selector} {`);
  expect(start, selector).toBeGreaterThanOrEqual(0);
  return css.slice(start, css.indexOf("}", start));
}

const DARK = ':global([data-mode="dark"])';

// 浅色模式下这几处是半透明白底；深色模式下文字是浅色，白底会让它看不清
describe("PieView styles in dark mode", () => {
  it("hovers category rows with the theme's hover color", () => {
    expect(block(PIE_CSS, `${DARK} .row:hover:not(:disabled)`)).toContain("var(--hover-bg)");
  });

  it("keeps the back button on a dark surface, also on hover", () => {
    expect(block(DRILL_CSS, `${DARK} .backBtn`)).toContain("var(--surface-2)");
    expect(block(DRILL_CSS, `${DARK} .backBtn:hover`)).toContain("var(--hover-bg)");
  });
});
