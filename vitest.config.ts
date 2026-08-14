// vitest 独立配置：vitest 4 不采用 vite.config.ts 内 test.environment（include 等字段却会读），
// 故测试配置独立存放并 mergeConfig 复用 vite 插件与别名
import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config";

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: "jsdom",
      include: ["src/**/*.{test,spec}.{ts,tsx}"],
      setupFiles: ["./src/shared/test/setup.ts"],
    },
  }),
);
