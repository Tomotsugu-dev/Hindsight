// 主题卡片上的色块用到的颜色。
//
// 自定义主题从默认主题的配色开始，mode 是 dark 时从深色主题的配色开始，theme.json 写了的变量再
// 盖上去。主题没写的颜色从应用的 tokens.css 里取，所以这里不另存一份颜色。不直接把 tokens.css
// 引进这一页：它和官网的 site.css 都在 :root 上定义变量，同名的会互相覆盖。
import type { ColorMode } from "@app/lib/theme";
import tokensCss from "@app/styles/tokens.css?raw";

/** 色块画出来要用的变量。 */
export const SWATCH_TOKENS = [
  "--sidebar-gradient",
  "--surface-app",
  "--surface-card",
  "--border-subtle",
  "--color-accent",
  "--text-strong",
  "--text-muted",
] as const;

/** 色块的颜色：变量名 → 值，值里的 var(...) 已经换成具体的值。 */
export function swatchColors(
  mode: ColorMode,
  tokens: Record<string, string>,
  css: string = tokensCss,
): Record<string, string> {
  const clean = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const vars = {
    ...declarations(clean, ":root"),
    ...(mode === "dark" ? declarations(clean, '[data-mode="dark"]') : {}),
    ...tokens,
  };
  return Object.fromEntries(SWATCH_TOKENS.map((name) => [name, resolve(vars[name] ?? "", vars)]));
}

/** 一个规则块里的变量：名 → 值。块到下一个顶格的 } 为止，和 tokens.css 的写法一致。 */
function declarations(css: string, selector: string): Record<string, string> {
  const start = css.indexOf(`${selector} {`);
  if (start < 0) return {};
  const body = css.slice(start + selector.length + 2).split("\n}")[0];
  const result: Record<string, string> = {};
  for (const declaration of body.split(";")) {
    const colon = declaration.indexOf(":");
    const name = declaration.slice(0, colon).trim();
    if (colon < 0 || !name.startsWith("--")) continue;
    result[name] = declaration.slice(colon + 1).trim().replace(/\s+/g, " ");
  }
  return result;
}

/** 把值里的 var(--x) 换成 vars 里的值。色块放在官网页面上，那里没有应用的变量可以引用。 */
function resolve(value: string, vars: Record<string, string>, depth = 0): string {
  if (depth > 5) return value;
  return value.replace(/var\(\s*(--[\w-]+)\s*(?:,\s*([^()]*))?\)/g, (_, name: string, fallback?: string) =>
    vars[name] !== undefined ? resolve(vars[name], vars, depth + 1) : (fallback ?? "").trim(),
  );
}
