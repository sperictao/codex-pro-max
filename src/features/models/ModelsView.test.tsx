// 模型配置域组件测试：渲染、三键应用、供应商增删改、预设流、目录/凭据/连通性/导入。
// commands 与 plugin-dialog 全量 mock；断言落点：invoke 参数与 DOM 状态（与 guard.test.tsx 同套路）

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import * as cmd from "@/shared/commands";
import type {
  CatalogView,
  CredentialEntry,
  ImportGroup,
  ModelConfigView,
  ProviderSchema,
} from "@/shared/types";
import { ModelsView } from "./ModelsView";

vi.mock("@/shared/commands", () => ({
  modelConfigView: vi.fn(),
  modelApply: vi.fn(),
  modelProviderSave: vi.fn(),
  modelProviderDelete: vi.fn(),
  modelPresetSave: vi.fn(),
  modelPresetDelete: vi.fn(),
  modelCatalogRefresh: vi.fn(),
  modelCredentialDescribe: vi.fn(),
  modelCredentialSet: vi.fn(),
  modelTestConnection: vi.fn(),
  modelRemoteList: vi.fn(),
  modelImportScan: vi.fn(),
  modelImportRun: vi.fn(),
}));

const askMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: (...args: unknown[]) => askMock(...args),
  open: vi.fn(),
}));

function makeProvider(over: Partial<ProviderSchema> = {}): ProviderSchema {
  return {
    route: "deepseek",
    name: "DeepSeek",
    baseURL: "https://api.deepseek.com/v1",
    envKey: "DEEPSEEK_API_KEY",
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
    ...over,
  };
}

const READY_CREDENTIAL: CredentialEntry = {
  name: "DEEPSEEK_API_KEY",
  info: { configured: true, source: "process", writable: false },
};

const MISSING_CREDENTIAL: CredentialEntry = {
  name: "DEEPSEEK_API_KEY",
  info: { configured: false, source: null, writable: true },
};

function makeCatalog(over: Partial<CatalogView> = {}): CatalogView {
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
          maxTokens: 64_000,
          input: ["text"],
          reasoning: true,
          reasoningLevels: ["low", "medium", "high"],
        },
      ],
    },
    ...over,
  };
}

function makeView(over: Partial<ModelConfigView> = {}): ModelConfigView {
  return {
    active: { model: "gpt-5-codex", provider: "deepseek", effort: "low" },
    providers: [makeProvider()],
    presets: [{ id: "p1", label: "Scout", model: "gpt-5.6-luna", provider: "", effort: "low" }],
    catalog: makeCatalog(),
    credentials: [READY_CREDENTIAL],
    efforts: ["none", "minimal", "low", "medium", "high", "xhigh", "max"],
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(cmd.modelConfigView).mockResolvedValue(makeView());
  vi.mocked(cmd.modelCatalogRefresh).mockResolvedValue(makeCatalog());
  vi.mocked(cmd.modelImportScan).mockResolvedValue([]);
  askMock.mockResolvedValue(true);
});

describe("模型配置视图渲染", () => {
  it("渲染当前模型表单、供应商行（Active 徽标 + 认证摘要）与预设行", async () => {
    render(<ModelsView />);

    expect(await screen.findByDisplayValue("gpt-5-codex")).toBeInTheDocument();
    expect(screen.getByText("DeepSeek")).toBeInTheDocument();
    expect(screen.getByText(/Env var: DEEPSEEK_API_KEY/)).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
    expect(screen.getByText("Scout")).toBeInTheDocument();
    expect(screen.getByText("gpt-5.6-luna · openai · low")).toBeInTheDocument();
  });

  it("provider 指向已删除供应商时补原值选项，不静默改值", async () => {
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ providers: [], active: { model: "m", provider: "deepseek", effort: "" } }),
    );
    render(<ModelsView />);

    // 触发器的可访问名即当前选中项的文本（Radix Select 把 value 文本放进触发器）
    expect(await screen.findByLabelText("Provider")).toHaveTextContent("deepseek (missing)");
  });

  it("目录未下载时给出引导文案而不是空表", async () => {
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ catalog: makeCatalog({ missing: true, providers: {} }) }),
    );
    render(<ModelsView />);

    expect(await screen.findByText("Catalog not downloaded")).toBeInTheDocument();
    expect(
      screen.getByText("Download the catalog to see context window and reasoning support."),
    ).toBeInTheDocument();
  });

  it("命中目录条目时展示上下文窗口与最大输出", async () => {
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ active: { model: "deepseek-reasoner", provider: "deepseek", effort: "high" } }),
    );
    render(<ModelsView />);

    expect(await screen.findByText("128K")).toBeInTheDocument();
    expect(screen.getByText("max output")).toBeInTheDocument();
    expect(screen.getByText("64K")).toBeInTheDocument();
    expect(screen.getByText("reasoning")).toBeInTheDocument();
  });

  it("模型 id 不在目录里时只提示，不拦截", async () => {
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ active: { model: "typo-model", provider: "deepseek", effort: "" } }),
    );
    render(<ModelsView />);

    expect(
      await screen.findByText(
        "This model id is not listed in the catalog for the selected provider.",
      ),
    ).toBeInTheDocument();
  });
});

describe("当前模型应用", () => {
  it("改模型 id 后 Apply：三键原样下发（provider=openai 也由后端删键）", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByDisplayValue("gpt-5-codex");

    const input = screen.getByLabelText("Model id");
    await user.clear(input);
    await user.type(input, "gpt-6");
    await user.click(screen.getAllByRole("button", { name: "Apply" })[0]);

    expect(cmd.modelApply).toHaveBeenCalledWith("gpt-6", "deepseek", "low");
  });

  it("预设 Apply：写入参数来自预设而非当前表单", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByText("Scout");

    await user.click(screen.getAllByRole("button", { name: "Apply" })[1]);

    expect(cmd.modelApply).toHaveBeenCalledWith("gpt-5.6-luna", "", "low");
  });

  it("切换供应商后 Apply 下发新路由", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByDisplayValue("gpt-5-codex");

    await user.click(screen.getByLabelText("Provider"));
    await user.click(await screen.findByRole("option", { name: "OpenAI (built-in)" }));
    await user.click(screen.getAllByRole("button", { name: "Apply" })[0]);

    expect(cmd.modelApply).toHaveBeenCalledWith("gpt-5-codex", "openai", "low");
  });

  it("活跃供应商的环境变量未设置时给出提示", async () => {
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ credentials: [MISSING_CREDENTIAL] }),
    );
    render(<ModelsView />);

    expect(
      await screen.findByText(
        "The active provider references an environment variable that is not set.",
      ),
    ).toBeInTheDocument();
  });
});

describe("供应商保存与删除", () => {
  it("新增供应商：认证选环境变量时只下发 envKey，且不透传无关字段", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Add Provider" }));
    await user.type(screen.getByLabelText("Provider id"), "kimi");
    await user.type(screen.getByLabelText("Display name"), "Kimi");
    await user.type(screen.getByLabelText("Base URL"), "https://api.kimi.com");
    // 认证方式默认「无鉴权」，显式切到环境变量后才会出现变量名输入框
    await user.click(screen.getByLabelText("Authentication"));
    // Radix 选中项后会把 scrollIntoView 副作用排到微任务之后，落点在 RTL 的 act
    // 之外，因此这条用例会打一条 "not wrapped in act(...)" 的 stderr 噪声；
    // 断言不受影响（上游在 jsdom 下的已知行为，非本域缺陷）。
    await user.click(await screen.findByRole("option", { name: "Environment variable name" }));
    await user.type(await screen.findByLabelText("Env var name"), "KIMI_API_KEY");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(cmd.modelProviderSave).toHaveBeenCalledTimes(1);
    const [payload, bearer, previousKeys] = vi.mocked(cmd.modelProviderSave).mock.calls[0];
    expect(payload.route).toBe("kimi");
    expect(payload.name).toBe("Kimi");
    expect(payload.baseURL).toBe("https://api.kimi.com");
    expect(payload.envKey).toBe("KIMI_API_KEY");
    // 新增 + env 模式 → 下发空串（直填密钥位显式清除；新供应商磁盘本无密钥）
    expect(bearer).toBe("");
    expect(previousKeys).toEqual([]);
  });

  it("编辑已有供应商：key 模式留空密钥下发 null 而不是清空", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ providers: [makeProvider({ envKey: null, hasBearerToken: true })] }),
    );
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    const [, bearer, previousKeys] = vi.mocked(cmd.modelProviderSave).mock.calls[0];
    expect(bearer).toBeNull();
    // 磁盘上读到的未知键名单要回传，后端据此删除"用户已移除"的透传键
    expect(previousKeys).toEqual([]);
  });

  it("env 模式保存时残留的直填密钥一并清除（认证二选一）", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ providers: [makeProvider({ hasBearerToken: true })] }),
    );
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    const [, bearer] = vi.mocked(cmd.modelProviderSave).mock.calls[0];
    expect(bearer).toBe("");
  });

  it("key 模式切到环境变量：下发空串清除磁盘密钥，而不是保持", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ providers: [makeProvider({ envKey: null, hasBearerToken: true })] }),
    );
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.click(screen.getByLabelText("Authentication"));
    await user.click(await screen.findByRole("option", { name: "Environment variable name" }));
    await user.type(await screen.findByLabelText("Env var name"), "KIMI_API_KEY");
    await user.click(screen.getByRole("button", { name: "Save" }));

    const [, bearer] = vi.mocked(cmd.modelProviderSave).mock.calls[0];
    expect(bearer).toBe("");
  });

  it("key 模式填入新密钥：下发新值，入库视图不带明文", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ providers: [makeProvider({ envKey: null, hasBearerToken: true })] }),
    );
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.type(screen.getByLabelText("API key (written to config.toml)"), "sk-brand-new");
    await user.click(screen.getByRole("button", { name: "Save" }));

    const [payload, bearer] = vi.mocked(cmd.modelProviderSave).mock.calls[0];
    expect(bearer).toBe("sk-brand-new");
    expect(payload.bearerToken).toBeUndefined();
  });

  it("新增供应商时把未知键名单回传（用于清除被删的透传键）", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ providers: [makeProvider({ unknownKeys: ["future_key"] })] }),
    );
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    const [, , previousKeys] = vi.mocked(cmd.modelProviderSave).mock.calls[0];
    expect(previousKeys).toEqual(["future_key"]);
  });

  it("凭据缺失时可在编辑面板写入用户级 .env", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ credentials: [MISSING_CREDENTIAL] }),
    );
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText("not set")).toBeInTheDocument();

    await user.type(within(dialog).getByLabelText("Value to store in ~/.codex/.env"), "sk-manual");
    await user.click(within(dialog).getByRole("button", { name: "Save to .env" }));

    expect(cmd.modelCredentialSet).toHaveBeenCalledWith("DEEPSEEK_API_KEY", "sk-manual");
  });

  it("凭据来自进程环境时标注只读，不提供写入入口", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = await screen.findByRole("dialog");
    expect(
      await within(dialog).findByText(/resolved from Process environment/),
    ).toBeInTheDocument();
    // 只读标注与来源在同一 <p> 的相邻文本节点里，按拼接文本匹配
    expect(within(dialog).getByText(/· read-only/)).toBeInTheDocument();
    expect(
      within(dialog).queryByLabelText("Value to store in ~/.codex/.env"),
    ).not.toBeInTheDocument();
  });

  it("未文档化的键在编辑面板里只读列出", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ providers: [makeProvider({ unknownKeys: ["supports_websockets_v2"] })] }),
    );
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Edit" }));

    expect(screen.getByText("Keys kept as-is")).toBeInTheDocument();
    expect(screen.getByText("supports_websockets_v2")).toBeInTheDocument();
  });

  it("删除供应商：确认后按 route 删除", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getAllByRole("button", { name: "More actions" })[0]);
    // Delete 收进 ⋯ 菜单（craft-spec 步骤 1..6）
    await user.click(await screen.findByRole("menuitem", { name: "Delete" }));

    expect(askMock).toHaveBeenCalledTimes(1);
    expect(cmd.modelProviderDelete).toHaveBeenCalledWith("deepseek");
  });

  it("删除供应商：确认弹窗取消则不删除", async () => {
    const user = userEvent.setup();
    askMock.mockResolvedValue(false);
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getAllByRole("button", { name: "More actions" })[0]);
    await user.click(await screen.findByRole("menuitem", { name: "Delete" }));

    expect(cmd.modelProviderDelete).not.toHaveBeenCalled();
  });

  it("删除后重新读取 config.toml（回落由后端决定，前端不复刻推导）", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByText("DeepSeek");
    expect(vi.mocked(cmd.modelConfigView).mock.calls.length).toBe(1);

    await user.click(screen.getAllByRole("button", { name: "More actions" })[0]);
    await user.click(await screen.findByRole("menuitem", { name: "Delete" }));

    expect(cmd.modelProviderDelete).toHaveBeenCalledWith("deepseek");
    // 删除后必须回读一次：model_provider 是否回落以磁盘为准
    await waitFor(() => expect(cmd.modelConfigView).toHaveBeenCalledTimes(2));
  });
});

describe("预设保存与删除", () => {
  it("存预设：只填名字，模型三字段取当前表单值", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByDisplayValue("gpt-5-codex");

    await user.click(screen.getByRole("button", { name: "Save current as preset" }));
    await user.type(screen.getByLabelText("Preset name"), "Main");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(cmd.modelPresetSave).toHaveBeenCalledWith({
      id: "",
      label: "Main",
      model: "gpt-5-codex",
      provider: "deepseek",
      effort: "low",
    });
  });

  it("删除预设：确认后按 id 删除", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByText("Scout");

    await user.click(screen.getAllByRole("button", { name: "More actions" })[1]);
    await user.click(await screen.findByRole("menuitem", { name: "Delete" }));

    expect(askMock).toHaveBeenCalledTimes(1);
    expect(cmd.modelPresetDelete).toHaveBeenCalledWith("p1");
  });
});

describe("目录刷新与选模型", () => {
  it("刷新目录：只更新目录，不动正在编辑的三键", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByDisplayValue("gpt-5-codex");

    const input = screen.getByLabelText("Model id");
    await user.clear(input);
    await user.type(input, "draft-model");
    await user.click(screen.getByRole("button", { name: "Refresh catalog" }));

    expect(cmd.modelCatalogRefresh).toHaveBeenCalledTimes(1);
    // 用户草稿未被目录刷新打断
    expect(screen.getByLabelText("Model id")).toHaveValue("draft-model");
  });

  it("从目录挑模型：写回模型 id 输入框", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByDisplayValue("gpt-5-codex");

    await user.click(screen.getByRole("button", { name: "Pick a model" }));
    await user.click(await screen.findByRole("button", { name: /DeepSeek Reasoner/ }));

    expect(screen.getByLabelText("Model id")).toHaveValue("deepseek-reasoner");
  });

  it("目录未下载时选模型面板显示引导", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelConfigView).mockResolvedValue(
      makeView({ catalog: makeCatalog({ missing: true, providers: {} }) }),
    );
    render(<ModelsView />);
    await screen.findByDisplayValue("gpt-5-codex");

    await user.click(screen.getByRole("button", { name: "Pick a model" }));

    expect(
      await screen.findByText("The model catalog has not been downloaded yet."),
    ).toBeInTheDocument();
  });
});

describe("连通性验证", () => {
  it("Test：展示端点、状态码与模型数", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelTestConnection).mockResolvedValue({
      ok: true,
      endpoint: "https://api.deepseek.com/v1/models",
      authenticated: true,
      status: 200,
      modelCount: 3,
      error: null,
    });
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Test" }));

    expect(cmd.modelTestConnection).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(/Reachable/)).toBeInTheDocument();
    expect(screen.getByText(/HTTP 200/)).toBeInTheDocument();
    expect(screen.getByText(/3 models/)).toBeInTheDocument();
  });

  it("Test：失败时展示错误原因而不是抛给用户", async () => {
    const user = userEvent.setup();
    vi.mocked(cmd.modelTestConnection).mockResolvedValue({
      ok: false,
      endpoint: "https://api.deepseek.com/v1/models",
      authenticated: true,
      status: 401,
      modelCount: null,
      error: "Endpoint returned HTTP 401",
    });
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Test" }));

    expect(await screen.findByText(/HTTP 401/)).toBeInTheDocument();
  });
});

describe("导入", () => {
  it("勾选候选后导入并回传 key", async () => {
    const user = userEvent.setup();
    const groups: ImportGroup[] = [
      {
        source: "opencode",
        entries: [
          {
            key: "opencode:relay",
            route: "relay",
            name: "Relay",
            baseURL: "https://relay.dev/v1",
            wireApi: null,
            envKey: "RELAY_KEY",
            credential: "env",
            models: ["m1"],
          },
        ],
      },
    ];
    vi.mocked(cmd.modelImportScan).mockResolvedValue(groups);
    vi.mocked(cmd.modelImportRun).mockResolvedValue({
      imported: 1,
      skipped: 0,
      failed: 0,
      literal: 0,
    });
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Import providers" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("switch"));
    await user.click(within(dialog).getByRole("button", { name: /Import 1 selected/ }));

    expect(cmd.modelImportRun).toHaveBeenCalledWith(["opencode:relay"]);
    await waitFor(() => expect(cmd.modelConfigView).toHaveBeenCalledTimes(2));
  });

  it("没有可导入内容时明确说明", async () => {
    const user = userEvent.setup();
    render(<ModelsView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Import providers" }));

    expect(
      await screen.findByText("Nothing to import was found on this machine."),
    ).toBeInTheDocument();
  });
});
