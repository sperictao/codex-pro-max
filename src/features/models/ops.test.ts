// features/models/ops 纯函数测试：视图推导、校验、目录判定与格式化。

import { describe, expect, it } from "vitest";
import type { CatalogView, ModelConfigView, ProviderSchema } from "@/shared/types";
import {
  BUILTIN_PROVIDER,
  EMPTY_CONFIG,
  authModeOf,
  catalogModel,
  effortSupported,
  emptyProvider,
  fmtTokens,
  modelStatus,
  normalizeProvider,
  prepareProvider,
  presetSummary,
  providerChoices,
  providerError,
  removeProvider,
  upsertProvider,
} from "./ops";
import { recordFromRows, rowsFromRecord } from "./HeadersEditor";

function provider(over: Partial<ProviderSchema> = {}): ProviderSchema {
  return { ...emptyProvider(), route: "deepseek", name: "DeepSeek", ...over };
}

/** 测试用直译 t：键即原文，{{var}} 占位按 opts 插值（与 i18n 行为对齐） */
const t = (key: string, opts?: Record<string, unknown>): string =>
  opts ? key.replace(/\{\{(\w+)\}\}/g, (_, k) => String(opts[k])) : key;

function view(over: Partial<ModelConfigView> = {}): ModelConfigView {
  return {
    ...EMPTY_CONFIG,
    active: { model: "gpt-5-codex", provider: "deepseek", effort: "low" },
    providers: [provider()],
    ...over,
  };
}

function catalog(over: Partial<CatalogView> = {}): CatalogView {
  return {
    missing: false,
    fetchedAt: 1_700_000_000,
    stale: false,
    providers: {
      deepseek: [
        {
          provider: "deepseek",
          id: "deepseek-reasoner",
          name: "DeepSeek Reasoner",
          context: 128_000,
          maxTokens: 65_536,
          input: ["text"],
          reasoning: true,
          reasoningLevels: ["low", "medium", "high"],
        },
      ],
    },
    ...over,
  };
}

describe("认证方式与摘要", () => {
  it("env 优先于直填密钥，都没有则无鉴权", () => {
    expect(authModeOf(provider({ envKey: "K" }))).toBe("env");
    expect(authModeOf(provider({ hasBearerToken: true }))).toBe("key");
    expect(authModeOf(provider())).toBe("none");
    // 两者同时存在时以 env_key 为准（与写盘语义一致）
    expect(authModeOf(provider({ envKey: "K", hasBearerToken: true }))).toBe("env");
  });
});

describe("providerChoices 保留悬空引用", () => {
  it("内置 openai 恒在首位", () => {
    const choices = providerChoices(view({ providers: [] }), t);
    expect(choices[0]).toEqual({ value: BUILTIN_PROVIDER, label: "OpenAI (built-in)" });
  });

  it("活跃 provider 已删除时补 missing 选项，不改值", () => {
    const choices = providerChoices(
      view({ providers: [], active: { model: "m", provider: "gone", effort: "" } }),
      t,
    );
    expect(choices.map((c) => c.value)).toEqual([BUILTIN_PROVIDER, "gone"]);
    expect(choices[1].label).toBe("gone (missing)");
  });

  it("活跃 provider 为内置或空时不补多余选项", () => {
    for (const active of ["", BUILTIN_PROVIDER]) {
      const choices = providerChoices(
        view({ active: { model: "m", provider: active, effort: "" } }),
        t,
      );
      expect(choices.map((c) => c.value)).toEqual([BUILTIN_PROVIDER, "deepseek"]);
    }
  });
});

describe("providerError 校验", () => {
  it("放行合法草稿", () => {
    expect(providerError(provider({ baseURL: "https://api.deepseek.com/v1", envKey: "K" }), "env")).toBeNull();
  });

  it("逐项拦截：空 id / 保留 id / 非法字符 / 空名 / 非法 URL", () => {
    expect(providerError(provider({ route: "" }), "none")).toBe("Provider id cannot be empty");
    expect(providerError(provider({ route: BUILTIN_PROVIDER }), "none")).toBe(
      "openai is the built-in provider id and cannot be recreated",
    );
    expect(providerError(provider({ route: "a.b" }), "none")).toBe(
      "Provider id may only contain letters, digits, '-' and '_'",
    );
    expect(providerError(provider({ baseURL: "https://x.dev", name: "  " }), "none")).toBe(
      "Provider name cannot be empty",
    );
    expect(providerError(provider({ baseURL: "" }), "none")).toBe("Base URL is required");
    expect(providerError(provider({ baseURL: "api.deepseek.com" }), "none")).toBe(
      "Base URL must start with http:// or https://",
    );
  });

  it("认证方式各自要求对应字段", () => {
    const base = provider({ baseURL: "https://x.dev" });
    expect(providerError({ ...base, envKey: "" }, "env")).toBe(
      "Environment variable name cannot be empty",
    );
    expect(providerError({ ...base, bearerToken: "" }, "key")).toBe("API key cannot be empty");
    // 已存密钥时留空合法（留空 = 保持磁盘原值）
    expect(providerError({ ...base, hasBearerToken: true }, "key")).toBeNull();
    // 无鉴权不要求任何凭据
    expect(providerError({ ...base, envKey: "K" }, "none")).toBeNull();
  });

  it("负数重试与超时被拦住", () => {
    const base = provider({ baseURL: "https://x.dev" });
    expect(providerError({ ...base, requestMaxRetries: -1 }, "none")).toBe(
      "requestMaxRetries cannot be negative",
    );
    expect(providerError({ ...base, startupTimeoutMs: -5 }, "none")).toBe(
      "startupTimeoutMs cannot be negative",
    );
  });
});

describe("normalizeProvider 落实「空 = 删键」", () => {
  it("裁剪空白并把空串折成 null", () => {
    const out = normalizeProvider(
      provider({ route: " kimi ", name: " Kimi ", baseURL: " https://x.dev ", envKey: " K " }),
      "env",
    );
    expect(out.route).toBe("kimi");
    expect(out.name).toBe("Kimi");
    expect(out.baseURL).toBe("https://x.dev");
    expect(out.envKey).toBe("K");
  });

  it("认证互斥：非当前方式的凭据一律清空", () => {
    const withBoth = provider({ envKey: "K", bearerToken: "sk-x" });
    expect(normalizeProvider(withBoth, "env").bearerToken).toBe("");
    expect(normalizeProvider(withBoth, "key").envKey).toBeNull();
    expect(normalizeProvider(withBoth, "none").envKey).toBeNull();
    expect(normalizeProvider(withBoth, "none").bearerToken).toBe("");
  });
});

describe("prepareProvider 密钥下发三态", () => {
  it("key 模式本次填了值 → 下发新值", () => {
    const out = prepareProvider(
      provider({ baseURL: "https://x.dev", bearerToken: "sk-new" }),
      "key",
      true,
    );
    expect(out.bearerToken).toBe("sk-new");
    expect(out.error).toBeNull();
  });

  it("key 模式留空 → 下发空串（对话框据此转 null = 保持磁盘原值）", () => {
    const out = prepareProvider(
      provider({ baseURL: "https://x.dev", hasBearerToken: true }),
      "key",
      false,
    );
    expect(out.bearerToken).toBe("");
    expect(out.error).toBeNull();
  });

  it("非 key 模式 → 空串（显式清除磁盘密钥）", () => {
    const env = prepareProvider(
      provider({ baseURL: "https://x.dev", envKey: "K", bearerToken: "sk-old" }),
      "env",
      false,
    );
    expect(env.bearerToken).toBe("");
    expect(env.payload.envKey).toBe("K");
    const none = prepareProvider(
      provider({ baseURL: "https://x.dev", envKey: "K", bearerToken: "sk-old" }),
      "none",
      false,
    );
    expect(none.payload.envKey).toBeNull();
    expect(none.bearerToken).toBe("");
  });
});

describe("upsertProvider / removeProvider", () => {
  it("同路由就地替换，不追加重复项", () => {
    const next = upsertProvider(view(), provider({ name: "Renamed" }), "deepseek");
    expect(next.providers).toHaveLength(1);
    expect(next.providers[0].name).toBe("Renamed");
  });

  it("新路由追加", () => {
    const next = upsertProvider(view(), provider({ route: "kimi", name: "Kimi" }), null);
    expect(next.providers.map((p) => p.route)).toEqual(["deepseek", "kimi"]);
  });

  it("改活跃供应商的路由键时同步跟随引用", () => {
    const next = upsertProvider(view(), provider({ route: "ds2" }), "deepseek");
    expect(next.active.provider).toBe("ds2");
  });

  it("改非活跃供应商的路由键不动引用", () => {
    const v = view({ active: { model: "m", provider: "other", effort: "" } });
    const next = upsertProvider(v, provider({ route: "ds2" }), "deepseek");
    expect(next.active.provider).toBe("other");
  });

  it("删除活跃供应商回落内置 openai，其余不动引用", () => {
    expect(removeProvider(view(), "deepseek").active.provider).toBe(BUILTIN_PROVIDER);
    const v = view({ active: { model: "m", provider: "other", effort: "" } });
    expect(removeProvider(v, "deepseek").active.provider).toBe("other");
    expect(removeProvider(view(), "deepseek").providers).toHaveLength(0);
  });
});

describe("目录判定", () => {
  it("未下载目录时一律 no-catalog，不误报", () => {
    expect(modelStatus({ ...catalog(), missing: true }, "deepseek", "deepseek-reasoner")).toEqual({
      kind: "no-catalog",
    });
    expect(modelStatus(EMPTY_CONFIG.catalog, "deepseek", "x")).toEqual({ kind: "no-catalog" });
  });

  it("目录不覆盖该路由时 unlisted（本地端点常见）", () => {
    expect(modelStatus(catalog(), "my-local", "llama")).toEqual({ kind: "unlisted" });
  });

  it("路由被覆盖但 id 不在其中时 unknown", () => {
    expect(modelStatus(catalog(), "deepseek", "typo-model")).toEqual({ kind: "unknown" });
  });

  it("命中时带出推理档", () => {
    const status = modelStatus(catalog(), "deepseek", "deepseek-reasoner");
    expect(status).toEqual({ kind: "listed", levels: ["low", "medium", "high"] });
  });

  it("effortSupported 只拦「目录明确说不支持」的情形", () => {
    const listed = modelStatus(catalog(), "deepseek", "deepseek-reasoner");
    expect(effortSupported(listed, "high")).toBe(true);
    expect(effortSupported(listed, "xhigh")).toBe(false);
    // 空 effort = 未设置，永远放行
    expect(effortSupported(listed, "")).toBe(true);
    // 目录缺失/未覆盖时不拦截（本地模型的档位无从得知）
    expect(effortSupported({ kind: "no-catalog" }, "xhigh")).toBe(true);
    expect(effortSupported({ kind: "unlisted" }, "xhigh")).toBe(true);
    expect(effortSupported({ kind: "unknown" }, "xhigh")).toBe(true);
  });

  it("catalogModel 按 id 精确查找", () => {
    expect(catalogModel(catalog(), "deepseek", "deepseek-reasoner")?.name).toBe(
      "DeepSeek Reasoner",
    );
    expect(catalogModel(catalog(), "deepseek", "")).toBeNull();
    expect(catalogModel(catalog(), "deepseek", "nope")).toBeNull();
    expect(catalogModel(catalog(), "unknown-route", "deepseek-reasoner")).toBeNull();
  });
});

describe("格式化", () => {
  it("fmtTokens 压缩到 K/M", () => {
    expect(fmtTokens(128_000)).toBe("128K");
    expect(fmtTokens(1_000_000)).toBe("1M");
    expect(fmtTokens(1_500_000)).toBe("1.5M");
    expect(fmtTokens(200_000)).toBe("200K");
    expect(fmtTokens(512)).toBe("512");
    expect(fmtTokens(null)).toBe("—");
    expect(fmtTokens(0)).toBe("—");
  });

  it("presetSummary 空 provider 回落内置名", () => {
    expect(presetSummary({ id: "1", label: "L", model: "m", provider: "", effort: "low" })).toBe(
      `m · ${BUILTIN_PROVIDER} · low`,
    );
    expect(
      presetSummary({ id: "1", label: "L", model: "m", provider: "ds", effort: "" }),
    ).toBe("m · ds");
  });
});


describe("请求头行 ⇄ 记录", () => {
  it("往返保序并丢弃空键", () => {
    const rows = rowsFromRecord({ A: "1", B: "2" });
    expect(rows).toEqual([
      { key: "A", value: "1" },
      { key: "B", value: "2" },
    ]);
    expect(recordFromRows(rows)).toEqual({ A: "1", B: "2" });
    expect(recordFromRows([{ key: "  ", value: "x" }])).toEqual({});
    expect(recordFromRows([{ key: " K ", value: " v " }])).toEqual({ K: "v" });
  });

  it("undefined 记录按空处理", () => {
    expect(rowsFromRecord(undefined)).toEqual([]);
  });
});
