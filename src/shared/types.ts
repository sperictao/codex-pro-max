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
  apply_mode: string;
  path: string;
  value_type: string;
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

// ============ 模型配置（config.toml 模型域；见 features/models） ============

/** 模型预设：一组可一键应用的模型组合（预设库在启动器配置，应用即写 config.toml） */
export interface ModelPreset {
  id: string;
  label: string;
  model: string;
  provider: string;
  effort: string;
}

/** config.toml 顶层三键；空 = 删键回落 Codex 默认 */
export interface ActiveModelView {
  model: string;
  provider: string;
  effort: string;
}

/**
 * 一个 `[model_providers.<id>]` 段的投影。
 * 托管字段强类型；`extra` 承载 Codex 新增/未文档化的键，编辑时原样保留。
 * 密钥明文永不回传：`bearerToken` 只写不读，存在性看 `hasBearerToken`。
 */
export interface ProviderSchema {
  route: string;
  name: string | null;
  baseURL: string | null;
  envKey: string | null;
  /** 只写：空串 = 清除磁盘上的 experimental_bearer_token */
  bearerToken?: string;
  hasBearerToken: boolean;
  wireApi: string | null;
  queryParams: Record<string, string>;
  httpHeaders: Record<string, string>;
  envHttpHeaders: Record<string, string>;
  requestMaxRetries: number | null;
  streamMaxRetries: number | null;
  streamIdleTimeoutMs: number | null;
  startupTimeoutMs: number | null;
  toolTimeoutSec: number | null;
  requiresOpenaiAuth: boolean | null;
  supportsWebsockets: boolean | null;
  /** 非托管键原样透传 */
  extra: unknown;
  /** 表里出现过但本应用不认识的键名（只读提示） */
  unknownKeys: string[];
}

/** 目录条目：models.dev 中一款模型的容量与能力声明 */
export interface CatalogEntry {
  provider: string;
  id: string;
  name: string;
  context: number | null;
  maxTokens: number | null;
  input: string[];
  reasoning: boolean;
  reasoningLevels: string[];
}

/** 目录视图：按已配置路由筛选后的条目 + 快照新鲜度 */
export interface CatalogView {
  /** 从未拉取或缓存损坏：UI 显示引导而不是空表 */
  missing: boolean;
  fetchedAt: number;
  stale: boolean;
  providers: Record<string, CatalogEntry[]>;
}

/** 环境变量引用的来源层；只有 user-env 是本应用能写的 */
export type CredentialSource = "process" | "user-env" | "project-env";

export interface CredentialInfo {
  configured: boolean;
  source: CredentialSource | null;
  writable: boolean;
}

export interface CredentialEntry {
  name: string;
  info: CredentialInfo;
}

/** 模型页视图：一次取齐，无密钥明文 */
export interface ModelConfigView {
  active: ActiveModelView;
  providers: ProviderSchema[];
  presets: ModelPreset[];
  catalog: CatalogView;
  credentials: CredentialEntry[];
  /** 可选推理档；档位表只有 Rust 一处事实来源 */
  efforts: string[];
}

/** 连通性验证结果：失败也是正常结果，不抛错 */
export interface ConnectionResult {
  ok: boolean;
  endpoint: string;
  authenticated: boolean;
  status: number | null;
  modelCount: number | null;
  error: string | null;
}

export interface RemoteModel {
  id: string;
}

/** 一个可导入候选；凭据明文永不进入本结构 */
export interface ImportCandidate {
  key: string;
  route: string;
  name: string;
  baseURL: string | null;
  wireApi: string | null;
  envKey: string | null;
  /** env = 环境变量引用 | literal = 来源持明文（值不导入） | none */
  credential: "env" | "literal" | "none";
  models: string[];
}

export interface ImportGroup {
  source: string;
  entries: ImportCandidate[];
}

export interface ImportRunResult {
  imported: number;
  skipped: number;
  failed: number;
  /** 选中条目里来源持明文密钥的数量（提示补 env_key） */
  literal: number;
}
