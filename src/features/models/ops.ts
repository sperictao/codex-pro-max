// features/models/ops：模型配置的纯函数层。
// 视图只持有"磁盘投影 + 本地草稿"，所有增删改的推导都放这里，便于单测。

import type {
  CatalogView,
  CredentialSource,
  ModelConfigView,
  ModelPreset,
  ProviderSchema,
} from "@/shared/types";

/** 内置供应商 id：Codex 默认即 openai，选中它等于删 model_provider 键 */
export const BUILTIN_PROVIDER = "openai";

/** Radix Select 禁止空字符串 value：以哨兵承载「未设置」，回写时映射回 "" */
export const NONE = "__none__";

/** 空配置占位：视图未加载完时使用，避免到处写 `view?.` */
export const EMPTY_CONFIG: ModelConfigView = {
  active: { model: "", provider: "", effort: "" },
  providers: [],
  presets: [],
  catalog: { missing: true, fetchedAt: 0, stale: true, providers: {} },
  credentials: [],
  efforts: [],
};

/** 新建供应商的空白草稿 */
export function emptyProvider(): ProviderSchema {
  return {
    route: "",
    name: null,
    baseURL: null,
    envKey: null,
    bearerToken: "",
    hasBearerToken: false,
    wireApi: null,
    queryParams: {},
    httpHeaders: {},
    envHttpHeaders: {},
    requestMaxRetries: null,
    streamMaxRetries: null,
    streamIdleTimeoutMs: null,
    startupTimeoutMs: null,
    toolTimeoutSec: null,
    requiresOpenaiAuth: null,
    supportsWebsockets: null,
    extra: {},
    unknownKeys: [],
  };
}

/** 认证方式三态：环境变量名 / 直填密钥 / 无鉴权（本地端点） */
export type AuthMode = "env" | "key" | "none";

export function authModeOf(p: ProviderSchema): AuthMode {
  if (p.envKey && p.envKey.trim()) return "env";
  if (p.hasBearerToken) return "key";
  return "none";
}

/** 认证摘要（列表行用）：不回显任何密钥内容 */
export function authSummary(
  p: ProviderSchema,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  if (p.envKey && p.envKey.trim()) return t("Env var: {{name}}", { name: p.envKey });
  if (p.hasBearerToken) return t("API key configured");
  return t("No auth");
}

/**
 * 供应商列表 + 当前活跃引用。
 * 活跃 provider 指向已删除的路由时保留原值（补一个「missing」选项），
 * 不静默改写用户配置——config.toml 才是事实来源。
 */
export function providerChoices(
  view: ModelConfigView,
  t: (key: string, opts?: Record<string, unknown>) => string,
): { value: string; label: string }[] {
  const options = [{ value: BUILTIN_PROVIDER, label: t("OpenAI (built-in)") }];
  for (const p of view.providers) {
    options.push({
      value: p.route,
      label: p.name ? `${p.name} (${p.route})` : p.route,
    });
  }
  const active = view.active.provider;
  if (active && active !== BUILTIN_PROVIDER && !view.providers.some((p) => p.route === active)) {
    options.push({ value: active, label: `${active} (${t("missing")})` });
  }
  return options;
}

/**
 * 把"界面上选的认证方式"叠加到草稿上：存储层只有 env_key 与
 * experimental_bearer_token 两个键，选中哪一种就等于另一种被清空。
 * 校验与下发都必须基于叠加后的形态，否则"输入框里没填"会被误判成非法。
 */
export function withAuthMode(provider: ProviderSchema, authMode: AuthMode): ProviderSchema {
  switch (authMode) {
    case "env":
      return { ...provider, bearerToken: "" };
    case "key":
      return { ...provider, envKey: null };
    default:
      return { ...provider, envKey: null, bearerToken: "" };
  }
}

/**
 * 草稿校验：返回错误 key（由调用方 t()），合法则 null。
 * `newKeyProvided` 表示密钥输入框里本次真的填了值——已有供应商留空是
 * 「保持磁盘原值」，不是「没有密钥」。
 */
export function providerError(
  provider: ProviderSchema,
  authMode: AuthMode,
  newKeyProvided = false,
): string | null {
  const route = (provider.route ?? "").trim();
  if (!route) return "Provider id cannot be empty";
  if (route === BUILTIN_PROVIDER) return "openai is the built-in provider id and cannot be recreated";
  if (!/^[A-Za-z0-9_-]+$/.test(route)) {
    return "Provider id may only contain letters, digits, '-' and '_'";
  }
  if (!(provider.name ?? "").trim()) return "Provider name cannot be empty";
  const base = (provider.baseURL ?? "").trim();
  if (!base) return "Base URL is required";
  if (!/^https?:\/\//.test(base)) return "Base URL must start with http:// or https://";
  if (authMode === "env" && !(provider.envKey ?? "").trim()) {
    return "Environment variable name cannot be empty";
  }
  if (
    authMode === "key" &&
    !(provider.bearerToken ?? "").trim() &&
    !provider.hasBearerToken &&
    !newKeyProvided
  ) {
    return "API key cannot be empty";
  }
  for (const [label, value] of [
    ["requestMaxRetries", provider.requestMaxRetries],
    ["streamMaxRetries", provider.streamMaxRetries],
    ["streamIdleTimeoutMs", provider.streamIdleTimeoutMs],
    ["startupTimeoutMs", provider.startupTimeoutMs],
  ] as const) {
    if (value !== null && value < 0) return `${label} cannot be negative`;
  }
  return null;
}

/**
 * 校验 + 归一化 + 密钥下发值的一步封装。
 *
 * `bearerToken` 的三态是这一段的关键：
 * - 非 key 模式 → `""`（显式清除磁盘上的 experimental_bearer_token）
 * - key 模式且本次填了值 → 新值
 * - key 模式但输入框留空 → `null`（保持磁盘原值；已有供应商留空只是"不改"）
 */
export function prepareProvider(
  provider: ProviderSchema,
  authMode: AuthMode,
  newKeyProvided: boolean,
): { payload: ProviderSchema; bearerToken: string | null; error: string | null } {
  const merged = withAuthMode(provider, authMode);
  return {
    payload: normalizeProvider(merged, authMode),
    bearerToken: authMode === "key" && newKeyProvided ? (merged.bearerToken ?? "") : "",
    error: providerError(merged, authMode, newKeyProvided),
  };
}

/** 归一化：把 UI 的"空 = 删键"语义落到下发给后端的载荷上 */
export function normalizeProvider(
  provider: ProviderSchema,
  authMode: AuthMode,
): ProviderSchema {
  const trim = (s: string | null) => {
    const v = (s ?? "").trim();
    return v === "" ? null : v;
  };
  return {
    ...provider,
    route: (provider.route ?? "").trim(),
    name: trim(provider.name),
    baseURL: trim(provider.baseURL),
    envKey: authMode === "env" ? trim(provider.envKey) : null,
    bearerToken: authMode === "key" ? (provider.bearerToken ?? "") : "",
    wireApi: trim(provider.wireApi),
  };
}

/**
 * 保存成功后就地更新配置：路由键保持不变时替换整段，否则追加。
 * 默认引用的供应商改路由键时同步跟随（否则会留下一个指向旧键的悬空引用）。
 */
export function upsertProvider(
  view: ModelConfigView,
  saved: ProviderSchema,
  previousRoute: string | null,
): ModelConfigView {
  const locate = previousRoute ?? saved.route;
  const index = view.providers.findIndex((p) => p.route === locate);
  const providers =
    index >= 0
      ? view.providers.map((p, i) => (i === index ? saved : p))
      : [...view.providers, saved];

  let provider = view.active.provider;
  if (previousRoute && provider === previousRoute && saved.route !== previousRoute) {
    provider = saved.route;
  }
  return { ...view, providers, active: { ...view.active, provider } };
}

/** 删除成功后就地更新配置；被删项是活跃引用时回落内置 openai */
export function removeProvider(view: ModelConfigView, route: string): ModelConfigView {
  const providers = view.providers.filter((p) => p.route !== route);
  const provider = view.active.provider === route ? BUILTIN_PROVIDER : view.active.provider;
  return { ...view, providers, active: { ...view.active, provider } };
}

// ============ 目录查询 ============

/** 在某路由的目录条目里按 id 找模型（大小写敏感，与请求身份一致） */
export function catalogModel(catalog: CatalogView, route: string, modelId: string) {
  const entries = catalog.providers[route];
  if (!entries || !modelId) return null;
  return entries.find((e) => e.id === modelId) ?? null;
}

/** 模型 id 在该路由目录里的状态 */
export type ModelStatus =
  /** 目录覆盖了该路由，但没有这个 id（可能是拼写错误或新模型） */
  | { kind: "unknown" }
  /** 目录里有这个 id */
  | { kind: "listed"; levels: string[] }
  /** 目录不覆盖该路由（本地自建端点很常见） */
  | { kind: "unlisted" }
  /** 目录尚未下载 */
  | { kind: "no-catalog" };

/**
 * 判定顺序很重要：目录缺失时一律 no-catalog（不能因为"没查到"就报错），
 * 目录存在但没有这个路由时 unlisted（本地自建端点很常见，不告警）。
 */
export function modelStatus(catalog: CatalogView, route: string, modelId: string): ModelStatus {
  if (catalog.missing) return { kind: "no-catalog" };
  const entries = catalog.providers[route];
  if (!entries) return { kind: "unlisted" };
  const hit = entries.find((e) => e.id === modelId);
  if (!hit) return { kind: "unknown" };
  return { kind: "listed", levels: hit.reasoningLevels };
}

/** 推理档在某模型上是否被支持；目录缺失/模型未列出时不拦截 */
export function effortSupported(status: ModelStatus, effort: string): boolean {
  if (!effort) return true;
  if (status.kind === "listed") return status.levels.includes(effort);
  return true;
}

// ============ 展示辅助 ============

/** 上下文/输出窗口的紧凑写法：128000 → 128K */
export function fmtTokens(value: number | null): string {
  if (value === null || value <= 0) return "—";
  if (value >= 1_000_000) {
    const millions = value / 1_000_000;
    return `${Number.isInteger(millions) ? millions : millions.toFixed(1)}M`;
  }
  if (value >= 1000) {
    const thousands = value / 1000;
    return `${Number.isInteger(thousands) ? thousands : thousands.toFixed(1)}K`;
  }
  return String(value);
}

/**
 * 凭据来源 → 展示用 i18n key。
 * 显式三态而非拼字符串：字典 key 就是原文，拼出来的 key 会绕过类型检查。
 */
export function credentialSourceKey(
  source: CredentialSource | null,
): "Process environment" | "User .env file" | "Project .env file" {
  switch (source) {
    case "process":
      return "Process environment";
    case "project-env":
      return "Project .env file";
    default:
      return "User .env file";
  }
}

/** 预设的一行摘要：model · provider · effort */
export function presetSummary(preset: ModelPreset): string {
  const provider = preset.provider || BUILTIN_PROVIDER;
  return [preset.model, provider, preset.effort].filter(Boolean).join(" · ");
}
