// shared/logger：前端统一日志入口。走浏览器 console（Tauri WebView 可经
// devtools/console 观察）；不落盘、不进 UI——Rust 侧 tauri-plugin-log 是唯一落盘通道
// （CONTEXT.md「日志只进文件不进 UI」边界）。统一 [codex-pro-max] 前缀便于过滤。

const PREFIX = "[codex-pro-max]";

function fmt(value: unknown): string {
  if (value instanceof Error) return value.message;
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

export const log = {
  info: (message: string) => console.info(`${PREFIX} ${message}`),
  warn: (context: string, err: unknown) => console.warn(`${PREFIX} ${context}: ${fmt(err)}`),
  error: (context: string, err: unknown) => console.error(`${PREFIX} ${context}: ${fmt(err)}`),
};