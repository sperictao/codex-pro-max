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
    rollupOptions: {
      output: {
        // 桌面应用从本地磁盘加载，拆包不是为了网络传输，而是让体积构成在每次构建里可见
        // （例如误 import 整个 radix barrel 会让 ui-kit 块立刻跳大），顺带消掉 Vite 的单块超限告警。
        // rolldown 只接受函数形式；按 node_modules 路径分组
        manualChunks(id: string) {
          if (!id.includes("node_modules")) return;
          if (/[\\/](react|react-dom|scheduler)[\\/]/.test(id)) return "react";
          if (/[\\/](i18next|react-i18next)[\\/]/.test(id)) return "i18n";
          if (/[\\/](radix-ui|@radix-ui|cmdk|sonner|lucide-react|class-variance-authority|clsx|tailwind-merge)[\\/]/.test(id))
            return "ui-kit";
          if (/[\\/]zustand[\\/]/.test(id)) return "state";
        },
      },
    },
  },
});
