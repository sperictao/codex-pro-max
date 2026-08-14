// shared/commands：集中式类型化 IPC——命令名全仓只出现在这里（ADR 0010）
// 参数名与 Rust 侧 #[tauri::command] 签名一一对应，行为与旧散点 invoke 完全一致

import { invoke } from "@tauri-apps/api/core";
import type {
  CustomParamPayload,
  DshStatus,
  FastctxApplyResult,
  FastctxStatus,
  GuardFileView,
  GuardView,
  LauncherConfig,
  ProcessInfo,
  SkillStatus,
  UpdateInfo,
  UpdaterConfigHealth,
  UpdaterHelpPaths,
} from "./types";

// ============ 配置 / 进程 ============
export const getBundledTaskboardPath = () => invoke<string | null>("get_bundled_taskboard_path");
export const loadConfig = () => invoke<LauncherConfig>("load_config");
export const updateSettings = (config: LauncherConfig) => invoke<void>("update_settings", { config });
export const startAll = (config: LauncherConfig) => invoke<void>("start_all", { config });
export const stopAll = () => invoke<void>("stop_all");
export const startTaskboard = (config: LauncherConfig) => invoke<void>("start_taskboard", { config });
export const stopTaskboard = () => invoke<void>("stop_taskboard");
export const startInjector = (config: LauncherConfig) => invoke<void>("start_injector", { config });
export const stopInjector = () => invoke<void>("stop_injector");
export const openTaskboard = (config: LauncherConfig) => invoke<void>("open_taskboard", { config });
export const quitCodex = () => invoke<void>("quit_codex");
export const getStatus = () => invoke<ProcessInfo[]>("get_status");
export const autostartIsEnabled = () => invoke<boolean>("autostart_is_enabled");
export const autostartSet = (enabled: boolean) => invoke<void>("autostart_set", { enabled });
export const checkNodeVersion = (nodePath: string) => invoke<string>("check_node_version", { nodePath });
export const checkCodexApp = (appPath: string) => invoke<boolean>("check_codex_app", { appPath });
export const detectCodexApp = () => invoke<string | null>("detect_codex_app");
export const validateTaskboardPath = (path: string) => invoke<boolean>("validate_taskboard_path", { path });
export const getLogDir = () => invoke<string>("get_log_dir");

// ============ 看守 ============
export const guardSetEnabled = (enabled: boolean) => invoke<void>("guard_set_enabled", { enabled });
export const guardGetView = () => invoke<GuardView>("guard_get_view");
export const guardSetValue = (id: string, value: unknown) => invoke<void>("guard_set_value", { id, value });
export const guardApply = (id: string) => invoke<void>("guard_apply", { id });
export const guardSetApplied = (id: string, applied: boolean) => invoke<void>("guard_set_applied", { id, applied });
export const guardSetLocked = (id: string, locked: boolean) => invoke<void>("guard_set_locked", { id, locked });
export const guardGetFiles = () => invoke<GuardFileView[]>("guard_get_files");
export const guardDetectFile = (id: string) => invoke<GuardFileView>("guard_detect_file", { id });
export const guardUpdateFile = (id: string, name: string, file: string) =>
  invoke<void>("guard_update_file", { id, name, file });
export const guardAddFile = (name: string, file: string, format: string) =>
  invoke<void>("guard_add_file", { name, file, format });
export const guardRemoveFile = (id: string) => invoke<void>("guard_remove_file", { id });
export const guardAddCustomParam = (param: CustomParamPayload, fileId: string) =>
  invoke<void>("guard_add_custom_param", { param, fileId });
export const guardRemoveCustomParam = (id: string) => invoke<void>("guard_remove_custom_param", { id });
export const guardGetSchemaFilePath = () => invoke<string>("guard_get_schema_file_path");
export const guardRelativizePickedPath = (absPath: string) =>
  invoke<string>("guard_relativize_picked_path", { absPath });

// ============ Skill ============
export const checkSkillStatus = (taskboardPath: string) =>
  invoke<SkillStatus>("check_skill_status", { taskboardPath });
export const installSkill = (taskboardPath: string) => invoke<string>("install_skill", { taskboardPath });

// ============ fastctx ============
export const fastctxDetect = () => invoke<FastctxStatus>("fastctx_detect");
export const fastctxInstall = () => invoke<void>("fastctx_install");
export const fastctxApply = () => invoke<FastctxApplyResult>("fastctx_apply");
export const fastctxUnapply = () => invoke<void>("fastctx_unapply");
export const fastctxOpenConsole = () => invoke<void>("fastctx_open_console");

// ============ dsh ============
export const dshDetect = () => invoke<DshStatus>("dsh_detect");
export const dshSetup = () => invoke<void>("dsh_setup");
export const dshStop = () => invoke<void>("dsh_stop");
export const dshUpdate = () => invoke<string>("dsh_update");
export const dshSetAutostart = (enabled: boolean) => invoke<void>("dsh_set_autostart", { enabled });

// ============ 更新 ============
export const getUpdaterConfigHealth = () => invoke<UpdaterConfigHealth>("get_updater_config_health");
export const getUpdaterHelpPaths = () => invoke<UpdaterHelpPaths>("get_updater_help_paths");
export const checkUpdate = () => invoke<UpdateInfo>("check_update");
export const installUpdate = (expectedVersion: string | null) =>
  invoke<string>("install_update", { expectedVersion });

// ============ 语言 ============
export const getResolvedLanguage = () => invoke<string>("get_resolved_language");
export const setLanguage = (setting: string) => invoke<void>("set_language", { setting });
