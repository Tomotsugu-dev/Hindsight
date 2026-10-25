import { describe, expect, it } from "vitest";
import { SWATCH_TOKENS, swatchColors } from "./swatch";

// 照 tokens.css 的写法：注释、跨行的值、引用别的变量
const CSS = `
:root {
  /* 强调色 */
  --color-accent: #6c5ce7;
  --sidebar-gradient:
    linear-gradient(180deg, #fff 0%, #eee 100%);
  --surface-app: #fafafa;
  --surface-card: var(--surface-app);
  --text-strong: #111;
}

[data-mode="dark"] {
  --color-accent: #8b7bf0;
  --surface-app: #0e0e11;
}
`;

describe("swatchColors", () => {
  it("fills missing swatch colors from the Default or Dark theme", () => {
    const light = swatchColors("light", { "--color-accent": "#ff3366" }, CSS);
    expect(light["--color-accent"]).toBe("#ff3366");
    expect(light["--surface-app"]).toBe("#fafafa");
    expect(light["--sidebar-gradient"]).toBe("linear-gradient(180deg, #fff 0%, #eee 100%)");
    // var() 换成具体的值：官网页面上没有应用的变量
    expect(light["--surface-card"]).toBe("#fafafa");

    const dark = swatchColors("dark", {}, CSS);
    expect(dark["--color-accent"]).toBe("#8b7bf0");
    expect(dark["--surface-app"]).toBe("#0e0e11");
    // 深色主题没写的，用默认主题的
    expect(dark["--text-strong"]).toBe("#111");
    expect(dark["--surface-card"]).toBe("#0e0e11");
  });

  // 应用改了 tokens.css 里变量的名字，色块会悄悄变空：这里用真的 tokens.css 查一遍
  it("finds every swatch color in the app's tokens.css", () => {
    for (const mode of ["light", "dark"] as const) {
      const colors = swatchColors(mode, {});
      for (const name of SWATCH_TOKENS) {
        expect(colors[name], `${mode} ${name}`).not.toBe("");
        expect(colors[name], `${mode} ${name}`).not.toContain("var(");
      }
    }
  });
});
