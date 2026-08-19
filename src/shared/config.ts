// shared/config：从 store 草稿构建 LauncherConfig（旧 readConfigFromUI 语义：
// 空 host/port/cdp 回落默认值；language/guardState 取 store 当前值；
// config 未加载时回落到旧静态 HTML 的初始值——auto_open 旧静态为勾选）

import type { GuardState, LauncherConfig } from "./types";

export function currentConfigDraft(s: {
  config: LauncherConfig | null;
  languageSetting: string;
  guardState: GuardState;
}): LauncherConfig {
  const c = s.config;
  return {
    taskboard_path: c?.taskboard_path ?? "",
    node_path: c?.node_path ?? "",
    codex_app_path: c?.codex_app_path ?? "",
    taskboard_host: c?.taskboard_host || "127.0.0.1",
    taskboard_port: c?.taskboard_port || 47823,
    cdp_port: c?.cdp_port || 9231,
    auto_open: c?.auto_open ?? true,
    separate_window_mode: c?.separate_window_mode ?? false,
    minimize_to_tray_on_close: c?.minimize_to_tray_on_close ?? false,
    language: s.languageSetting,
    codex_guard: s.guardState,
    dsh_admin_cap_domain: c?.dsh_admin_cap_domain ?? "",
    dsh_use_cap_domain: c?.dsh_use_cap_domain ?? "",
    dsh_extra_allowed_logins: c?.dsh_extra_allowed_logins ?? "",
  };
}
