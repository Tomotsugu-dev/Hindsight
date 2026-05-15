// Vite config for the Hindsight embeddable web demo.
//
// 把 src/pages/* 等真组件构建成纯 web bundle，通过 alias 把所有 Tauri 依赖
// 重定向到 web/demo-src/tauri/* 的 mock 实现。
//
// 用法：
//   npm run build:demo   → 产物到 web/demo/
//   landing page (web/index.html) 里 iframe src="/demo/" 加载
//
// 物理布局（web/ 自包含 + src/ 不被污染）：
//   src/         主 Tauri 应用源（被 demo import，不修改）
//   web/         所有 web 资产
//   ├── vite.config.ts   本文件
//   ├── demo-src/        demo 源码（mock + 入口）—— 通过 @app alias 引用主 src
//   └── demo/            build 产物

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

// __dirname 这里指 web/，repoRoot 上跳一层
const webDir = __dirname;
const repoRoot = path.resolve(webDir, "..");
const demoSrc = path.resolve(webDir, "demo-src");
const mainSrc = path.resolve(repoRoot, "src");
const tauriMock = path.resolve(demoSrc, "tauri");

export default defineConfig({
  plugins: [react()],

  // demo 自己的 HTML entry 在 web/demo-src/
  root: demoSrc,

  // 生产部署在 hindsight.kyosweb.com/demo/，需要 base URL
  base: "/demo/",

  // 把主仓库的 node_modules 暴露给 demo（demo 本身不装独立依赖）
  cacheDir: path.resolve(repoRoot, "node_modules/.vite-demo"),

  resolve: {
    alias: [
      // @app/ → 主仓库 src/，让 demo 干净地 import 主应用代码
      { find: "@app", replacement: mainSrc },

      // 把所有 Tauri 入口替换成 mock
      { find: "@tauri-apps/api/core", replacement: path.resolve(tauriMock, "core.ts") },
      { find: "@tauri-apps/api/event", replacement: path.resolve(tauriMock, "event.ts") },
      { find: "@tauri-apps/api/window", replacement: path.resolve(tauriMock, "api-window.ts") },
      { find: "@tauri-apps/api/app", replacement: path.resolve(tauriMock, "api-app.ts") },
      { find: "@tauri-apps/plugin-dialog", replacement: path.resolve(tauriMock, "plugin-dialog.ts") },
      { find: "@tauri-apps/plugin-opener", replacement: path.resolve(tauriMock, "plugin-opener.ts") },
      { find: "@tauri-apps/plugin-os", replacement: path.resolve(tauriMock, "plugin-os.ts") },
      { find: "@tauri-apps/plugin-process", replacement: path.resolve(tauriMock, "plugin-process.ts") },
      { find: "@tauri-apps/plugin-updater", replacement: path.resolve(tauriMock, "plugin-updater.ts") },

      // 替换主 api 文件 —— 任何 import "../api/hindsight" 或 "@app/api/hindsight"
      // 都重定向到 mock。正则匹配，覆盖各种深度的相对 / 别名路径。
      {
        find: /^.*\/api\/hindsight$/,
        replacement: path.resolve(demoSrc, "api-mock.ts"),
      },

    ],
  },

  build: {
    outDir: path.resolve(webDir, "demo"),
    emptyOutDir: true,
    sourcemap: false,
    minify: "esbuild",
    target: "es2022",
    rollupOptions: {
      input: path.resolve(demoSrc, "index.html"),
    },
    assetsDir: "assets",
    chunkSizeWarningLimit: 600,
  },

  // Demo 不开 dev server（开发期间直接 build + 在 web/ 跑 python http.server 看）
  server: {
    port: 5174,
    strictPort: false,
  },
});
