// 跨域共享状态与配置词汇（ADR 0009：依赖单向，域模块 → state/工具）
// 只有被两个以上域引用的状态才住这里；域内状态留在各自模块

import type {
  CodexGuardState as GeneratedCodexGuardState,
} from "./generated/guard-contracts";

export type { GuardParamState } from "./generated/guard-contracts";
export type CodexGuardState = Required<Pick<GeneratedCodexGuardState, "enabled" | "params">>;

export interface LauncherConfig {
  taskboard_path: string;
  node_path: string;
  codex_app_path: string;
  taskboard_port: number;
  taskboard_host: string;
  cdp_port: number;
  auto_open: boolean;
  separate_window_mode: boolean;
  minimize_to_tray_on_close: boolean;
  language: string;
  codex_guard: CodexGuardState;
}

export const state = {
  guardState: { enabled: false, params: {} } as CodexGuardState,
  // 语言设置（"system" | "en" | "zh-CN"），fillConfigUI 灌入、语言卡片改写
  languageSetting: "system",
};
