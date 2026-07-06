import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // 依赖预构建只从主入口扫。默认会把 web/demo-src/index.html 也当入口,
  // 那边的 @app/* alias 只在 web/vite.config.ts(demo 构建)里定义,
  // dev 启动会刷一屏 "could not be resolved" 假警告。
  optimizeDeps: {
    entries: ["index.html"],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    // 1420、7420 先后被 Hyper-V/WinNAT 动态保留区吃掉(EACCES)→ 迁 5420(当前
    // 保留区外),并建议用 netsh 管理员保留钉死(见 README/或 git log)。改这里必须同步改
    // src-tauri/tauri.conf.json 的 build.devUrl，否则 Tauri 连不上 dev server。
    port: 5420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 5421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
