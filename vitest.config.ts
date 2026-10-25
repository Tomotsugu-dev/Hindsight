import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// 纯函数 / 纯逻辑单测：node 环境足够，不挂 jsdom。
// 组件 / hook 测试（需要 DOM、renderHook）后续再单独加 environment。
export default defineConfig({
  // web/demo-src 用 @app 引用主应用的 src/，和 web/vite.config.ts 一样
  resolve: {
    alias: { "@app": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  test: {
    environment: "node",
    include: ["src/**/*.{test,spec}.{ts,tsx}", "web/**/*.test.{js,ts}"],
  },
});
