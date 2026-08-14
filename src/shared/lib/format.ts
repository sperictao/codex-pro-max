import { currentLanguage } from "../i18n";

// 时间戳按界面语言 locale 格式化，24 小时制；null 显示 "—"（与旧 fmtTs 一致）
export function fmtTs(ts: number | null): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString(currentLanguage() === "zh-CN" ? "zh-CN" : "en-US", { hour12: false });
}
