import "./style.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { getStoredFamily, getStoredTheme, resolveDataTheme } from "./theme";

// 主题必须在首次绘制前同步应用，避免首屏按默认色绘制后再闪切（沿用旧 init 的约束）
document.documentElement.dataset.theme = resolveDataTheme(
  getStoredTheme(localStorage.getItem("theme")),
  getStoredFamily(localStorage.getItem("theme-family")),
  window.matchMedia("(prefers-color-scheme: dark)").matches,
);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
