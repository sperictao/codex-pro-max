// shared/types：IPC 载荷与领域视图类型（镜像 Rust 侧结构；保行为重写，字段与旧模块一致）

export interface LauncherConfig {
  taskboard_path: string;
  node_path: string;
  codex_app_path: string;
  taskboard_host: string;
  taskboard_port: number;
  cdp_port: number;
  auto_open: boolean;
  separate_window_mode: boolean;
  minimize_to_tray_on_close: boolean;
  language: string;
  codex_guard: GuardState;
}

export interface GuardState {
  enabled: boolean;
  params: Record<string, { locked?: boolean } | undefined>;
}

export type ProcessStatus = "stopped" | "starting" | "running" | "stopping" | "failed";

export interface ProcessInfo {
  name: string;
  status: ProcessStatus;
  pid: number | null;
  message: string;
}

export interface SkillStatus {
  state: "installed" | "not-installed" | "mismatch";
  detail: string;
  targetPath: string;
}

export interface GuardParamView {
  id: string;
  label: string;
  description: string;
  applyMode: string;
  valueType: string;
  path: string;
  default: unknown;
  value: unknown;
  applied: boolean;
  locked: boolean;
  actual: string | null;
  status: "match" | "drift" | "missing" | "error";
  error: string | null;
  lastChecked: number | null;
  lastRestored: number | null;
  custom: boolean;
}

export interface GuardGroupView {
  id: string;
  name: string;
  file: string;
  format: string;
  builtin: boolean;
  error: string | null;
  params: GuardParamView[];
}

export interface GuardFileView {
  id: string;
  name: string;
  file: string;
  format: string;
  builtin: boolean;
  detection: { path: string | null; at: number } | null;
}

export interface GuardView {
  enabled: boolean;
  groups: GuardGroupView[];
}

export interface CustomParamPayload {
  id: string;
  label: string;
  description: string;
  file: string;
  applyMode: string;
  path: string;
  valueType: string;
  default: unknown;
  custom: boolean;
}

export interface FastctxStatus {
  installed: boolean;
  version: string | null;
  integrated: boolean;
  latestVersion: string | null;
}

export interface FastctxApplyResult {
  selfCheckPassed: boolean;
  selfCheckOutput: string;
}

export interface DshStatus {
  nodeAvailable: boolean;
  dshInstalled: boolean;
  dshVersion: string | null;
  latestVersion: string | null;
  dshRunning: boolean;
  tailscaleInstalled: boolean;
  tailscaleOnline: boolean;
  hostname: string | null;
  url: string | null;
  magicDnsEnabled: boolean;
  serveConfigured: boolean;
  proxyRunning: boolean;
  proxyConfigured: boolean;
  autostartEnabled: boolean;
  error: string | null;
}

export interface DshStepEvent {
  index: number;
  id: string;
  state: "running" | "done" | "failed" | "skipped" | "pending";
  detail: string | null;
  problem: string | null;
  solution: string | null;
}

export interface UpdaterConfigHealth {
  configured: boolean;
  message: string;
}

export interface UpdaterHelpPaths {
  docsPath: string;
  templatePath: string;
}

export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string | null;
  hasUpdate: boolean;
  releaseNotes: string | null;
  message: string | null;
}

export interface DownloadProgress {
  stage: string;
  version: string;
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number | null;
  attempt: number;
  maxAttempts: number;
}
