// 官网的主题页 /themes/。和 demo 分开构建：base 不一样，也用不到 demo 那些 Tauri 的替身。
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

const themesSrc = __dirname;
const repoRoot = path.resolve(themesSrc, "../..");

export default defineConfig({
  plugins: [react()],
  root: themesSrc,
  base: "/themes/",
  cacheDir: path.resolve(repoRoot, "node_modules/.vite-themes"),
  resolve: {
    alias: [{ find: "@app", replacement: path.resolve(repoRoot, "src") }],
  },
  build: {
    outDir: path.resolve(repoRoot, "web/themes"),
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: { input: path.resolve(themesSrc, "index.html") },
  },
});
