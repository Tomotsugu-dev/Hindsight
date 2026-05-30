import { defineConfig } from "vitest/config";

// 纯函数 / 纯逻辑单测：node 环境足够，不挂 jsdom。
// 组件 / hook 测试（需要 DOM、renderHook）后续再单独加 environment。
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
});
