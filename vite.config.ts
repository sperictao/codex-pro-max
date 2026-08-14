import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  root: ".",
  clearScreen: false,
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // src-tauri/target 与 vendor 不是前端资源；Windows 上 chokidar
      // 扫到构建产物 exe（Defender 锁定）会 EBUSY 崩掉整个 dev server
      ignored: ["**/src-tauri/**", "**/vendor/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  optimizeDeps: {
    // 默认会递归扫全仓 .html（含 vendor/dashi-taskboard 的入口），导致预打包
    // 把 vendor 自己的 react-dom/scheduler 捆进来 → 双 React 实例、dispatcher 为 null。
    // 显式限定入口只扫壳前端（smoke.html 是冒烟 harness，不随发布）
    entries: ["index.html", "smoke.html"],
  },
  build: {
    target: "es2022",
    minify: "esbuild",
    sourcemap: false,
    outDir: "dist",
  },
});
