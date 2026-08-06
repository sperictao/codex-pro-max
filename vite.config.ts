import { defineConfig } from "vite";

export default defineConfig({
  root: ".",
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // src-tauri/target 与 vendor 不是前端资源；Windows 上 chokidar
      // 扫到构建产物 exe（Defender 锁定）会 EBUSY 崩掉整个 dev server
      ignored: ["**/src-tauri/**", "**/vendor/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    minify: "esbuild",
    sourcemap: false,
    outDir: "dist",
  },
});
