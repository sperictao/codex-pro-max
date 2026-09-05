// shared/commands：集中式类型化 IPC——命令名全仓只出现在这里（ADR 0010）
// 参数名与 Rust 侧 #[tauri::command] 签名一一对应，行为与旧散点 invoke 完全一致

import { invoke } from "@tauri-apps/api/core";
import { log } from "./logger";
import type {
  CustomParamPayload,
  FastctxApplyResult,
  FastctxStatus,
  GuardFileView,
  GuardView,
  LauncherConfig,
  ModelConfigView,
  ModelPreset,
  ProcessInfo,
  SkillStatus,
  UpdateInfo,
  UpdaterConfigHealth,
  UpdaterHelpPaths,
} from "./types";

/// 唯一 invoke 出口：失败统一记一条前端日志（带命令名），再原样抛给调用方 toast。
/// 命令名全仓只出现在这里（ADR 0010），新命令必须经此包装。
async function invokeTyped<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (e) {
    log.error(`invoke ${command}`, e);
    throw e;
  }
}

// ============ 配置 / 进程 ============
export const getBundledTaskboardPath = () => invokeTyped<string | null>("get_bundled_taskboard_path");
export const loadConfig = () => invokeTyped<LauncherConfig>("load_config");
export const updateSettings = (config: LauncherConfig) => invokeTyped<void>("update_settings", { config });
export const startAll = (config: LauncherConfig) => invokeTyped<void>("start_all", { config });
export const stopAll = () => invokeTyped<void>("stop_all");
export const startTaskboard = (config: LauncherConfig) => invokeTyped<void>("start_taskboard", { config });
export const stopTaskboard = () => invokeTyped<void>("stop_taskboard");
export const startInjector = (config: LauncherConfig) => invokeTyped<void>("start_injector", { config });
export const stopInjector = () => invokeTyped<void>("stop_injector");
export const openTaskboard = (config: LauncherConfig) => invokeTyped<void>("open_taskboard", { config });
export const quitCodex = () => invokeTyped<void>("quit_codex");
export const getStatus = () => invokeTyped<ProcessInfo[]>("get_status");
export const autostartIsEnabled = () => invokeTyped<boolean>("autostart_is_enabled");
export const autostartSet = (enabled: boolean) => invokeTyped<void>("autostart_set", { enabled });
export const checkNodeVersion = (nodePath: string) => invokeTyped<string>("check_node_version", { nodePath });
export const checkCodexApp = (appPath: string) => invokeTyped<boolean>("check_codex_app", { appPath });
export const detectCodexApp = () => invokeTyped<string | null>("detect_codex_app");
export const validateTaskboardPath = (path: string) => invokeTyped<boolean>("validate_taskboard_path", { path });
export const getLogDir = () => invokeTyped<string>("get_log_dir");

// ============ 看守 ============
export const guardSetEnabled = (enabled: boolean) => invokeTyped<void>("guard_set_enabled", { enabled });
export const guardGetView = () => invokeTyped<GuardView>("guard_get_view");
export const guardSetValue = (id: string, value: unknown) => invokeTyped<void>("guard_set_value", { id, value });
export const guardApply = (id: string) => invokeTyped<void>("guard_apply", { id });
export const guardSetApplied = (id: string, applied: boolean) => invokeTyped<void>("guard_set_applied", { id, applied });
export const guardSetLocked = (id: string, locked: boolean) => invokeTyped<void>("guard_set_locked", { id, locked });
export const guardGetFiles = () => invokeTyped<GuardFileView[]>("guard_get_files");
export const guardDetectFile = (id: string) => invokeTyped<GuardFileView>("guard_detect_file", { id });
export const guardUpdateFile = (id: string, name: string, file: string) =>
  invokeTyped<void>("guard_update_file", { id, name, file });
export const guardAddFile = (name: string, file: string, format: string) =>
  invokeTyped<void>("guard_add_file", { name, file, format });
export const guardRemoveFile = (id: string) => invokeTyped<void>("guard_remove_file", { id });
export const guardAddCustomParam = (param: CustomParamPayload, fileId: string) =>
  invokeTyped<void>("guard_add_custom_param", { param, fileId });
export const guardRemoveCustomParam = (id: string) => invokeTyped<void>("guard_remove_custom_param", { id });
export const guardRemoveConfig = (id: string) => invokeTyped<void>("guard_remove_config", { id });
export const guardGetSchemaFilePath = () => invokeTyped<string>("guard_get_schema_file_path");
export const guardRelativizePickedPath = (absPath: string) =>
  invokeTyped<string>("guard_relativize_picked_path", { absPath });

// ============ Skill ============
export const checkSkillStatus = (taskboardPath: string) =>
  invokeTyped<SkillStatus>("check_skill_status", { taskboardPath });
export const installSkill = (taskboardPath: string) => invokeTyped<string>("install_skill", { taskboardPath });

// ============ fastctx ============
export const fastctxDetect = () => invokeTyped<FastctxStatus>("fastctx_detect");
export const fastctxInstall = () => invokeTyped<void>("fastctx_install");
export const fastctxApply = () => invokeTyped<FastctxApplyResult>("fastctx_apply");
export const fastctxUnapply = () => invokeTyped<void>("fastctx_unapply");
export const fastctxOpenConsole = () => invokeTyped<void>("fastctx_open_console");

// ============ 更新 ============
export const getUpdaterConfigHealth = () => invokeTyped<UpdaterConfigHealth>("get_updater_config_health");
export const getUpdaterHelpPaths = () => invokeTyped<UpdaterHelpPaths>("get_updater_help_paths");
export const checkUpdate = () => invokeTyped<UpdateInfo>("check_update");
export const installUpdate = (expectedVersion: string | null) =>
  invokeTyped<string>("install_update", { expectedVersion });

// ============ 模型配置 ============
// 三键语义统一「空 = 回落默认（删键）」；provider 为 openai 与空等价（内置默认）
export const modelConfigView = () => invokeTyped<ModelConfigView>("model_config_view");
export const modelApply = (model: string, provider: string, effort: string) =>
  invokeTyped<void>("model_apply", { model, provider, effort });
export const modelProviderSave = (p: {
  id: string;
  name: string;
  baseUrl: string;
  envKey: string;
  bearerToken: string;
}) => invokeTyped<void>("model_provider_save", p);
export const modelProviderDelete = (id: string) =>
  invokeTyped<void>("model_provider_delete", { id });
export const modelPresetSave = (preset: ModelPreset) =>
  invokeTyped<void>("model_preset_save", { preset });
export const modelPresetDelete = (id: string) => invokeTyped<void>("model_preset_delete", { id });

// ============ 语言 ============
export const getResolvedLanguage = () => invokeTyped<string>("get_resolved_language");
export const setLanguage = (setting: string) => invokeTyped<void>("set_language", { setting });