import { readdirSync, readFileSync } from "node:fs";
import { expect, it } from "vitest";
import config from "./vite.config";

const src = new URL("../src/", import.meta.url);

/** src 下所有的源码文件（不含测试），路径相对 src。 */
function sourceFiles() {
  return readdirSync(src, { recursive: true }).filter(
    (file) => /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file),
  );
}

// 演示把应用的页面打进网页。应用引用的每个 Tauri 模块，都要在 vite.config.ts 里换成替身：
// 真模块在浏览器里调不通，按钮点了没反应（导出就是这样坏的）。
// 插件窗口是另一个入口，演示不加载它。
it("has a stand-in for every Tauri module the app imports", () => {
  const aliases = config.resolve.alias;
  const missing = new Set();
  for (const file of sourceFiles()) {
    if (file.startsWith("plugin-window")) continue;
    const text = readFileSync(new URL(file, src), "utf8");
    for (const [, name] of text.matchAll(/from "(@tauri-apps\/[^"]+)"/g)) {
      const covered = aliases.some((a) =>
        typeof a.find === "string" ? a.find === name : a.find.test(name),
      );
      if (!covered) missing.add(name);
    }
  }
  expect([...missing]).toEqual([]);
});
