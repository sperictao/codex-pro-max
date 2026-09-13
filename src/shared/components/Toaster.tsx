import type { CSSProperties } from "react";
import { Toaster as SonnerToaster } from "sonner";

// Toast 由 sonner 拥有队列、定时、堆叠与滑动关闭（0b：store 的 toast 队列已删除）。
// 样式按主题 token 重贴（craft-spec R1/R2/R9）；类型色在 style.css 里按 [data-type] 映射到状态 token。
// duration 3300 冻结旧观感：3s 起淡出、3.3s 移除。
const TOKEN_STYLE = {
  "--normal-bg": "var(--popover)",
  "--normal-text": "var(--popover-foreground)",
  "--normal-border": "var(--border)",
  "--border-radius": "var(--radius)",
} as CSSProperties;

export function Toaster() {
  return (
    <SonnerToaster
      position="bottom-right"
      duration={3300}
      style={TOKEN_STYLE}
      className="toaster group"
    />
  );
}
