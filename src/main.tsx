import "./style.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { getResolvedLanguage } from "./shared/commands";
import { initI18n, type ResolvedLanguage } from "./shared/i18n";
import { getStoredFamily, getStoredTheme, resolveDataTheme } from "./shared/theme";

// 主题必须在首次绘制前同步应用，避免首屏按默认色绘制后再闪切（沿用旧 init 的约束）
document.documentElement.dataset.theme = resolveDataTheme(
  getStoredTheme(localStorage.getItem("theme")),
  getStoredFamily(localStorage.getItem("theme-family")),
  window.matchMedia("(prefers-color-scheme: dark)").matches,
);

// i18n 在首次渲染前就绪：语言在 Rust 启动时已解析好，失败回落英文（与旧 init 一致）
async function bootstrap(): Promise<void> {
  let lang: ResolvedLanguage = "en";
  try {
    lang = (await getResolvedLanguage()) === "zh-CN" ? "zh-CN" : "en";
  } catch {
    /* 回落英文 */
  }
  await initI18n(lang);
  document.documentElement.lang = lang;
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void bootstrap();
