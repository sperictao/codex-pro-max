// 模型配置域组件测试：渲染、当前模型三键应用、预设应用/保存、供应商保存/删除确认流。
// commands 与 plugin-dialog 全量 mock；断言落点：invoke 参数、DOM 状态（与 guard.test.tsx 同套路）

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import type { ModelConfigView, ModelProviderView } from "@/shared/types";
import { ModelView } from "./ModelView";

vi.mock("@/shared/commands", () => ({
  modelConfigView: vi.fn(),
  modelApply: vi.fn(),
  modelProviderSave: vi.fn(),
  modelProviderDelete: vi.fn(),
  modelPresetSave: vi.fn(),
  modelPresetDelete: vi.fn(),
}));

const askMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: (...args: unknown[]) => askMock(...args),
  open: vi.fn(),
}));

function makeProvider(over: Partial<ModelProviderView>): ModelProviderView {
  return {
    id: "deepseek",
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
    envKey: "DEEPSEEK_API_KEY",
    bearerToken: "",
    active: true,
    ...over,
  };
}

function makeView(over: Partial<ModelConfigView>): ModelConfigView {
  return {
    model: "gpt-5-codex",
    provider: "deepseek",
    effort: "low",
    providers: [makeProvider({})],
    presets: [{ id: "p1", label: "Scout", model: "gpt-5.6-luna", provider: "", effort: "low" }],
    ...over,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState(initialState, true);
  vi.mocked(cmd.modelConfigView).mockResolvedValue(makeView({}));
  askMock.mockResolvedValue(true);
});

describe("模型配置视图渲染", () => {
  it("渲染当前模型表单、供应商卡（Active 徽标 + 认证摘要）与预设卡", async () => {
    render(<ModelView />);

    expect(await screen.findByDisplayValue("gpt-5-codex")).toBeInTheDocument();
    expect(screen.getByText("DeepSeek")).toBeInTheDocument();
    expect(screen.getByText("Env var: DEEPSEEK_API_KEY")).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
    expect(screen.getByText("Scout")).toBeInTheDocument();
    // 预设行展示 model · provider · effort
    expect(screen.getByText("gpt-5.6-luna · openai · low")).toBeInTheDocument();
  });

  it("provider 指向已删除供应商时补原值选项，不静默改值", async () => {
    vi.mocked(cmd.modelConfigView).mockResolvedValue(makeView({ providers: [] }));
    render(<ModelView />);

    // 选中 option 文本为「id (missing)」，值仍是原 id（testing-library 对 select 匹配 option 文本）
    expect(await screen.findByText("deepseek (missing)")).toBeInTheDocument();
  });
});

describe("当前模型应用", () => {
  it("改模型 id 后 Apply：三键原样下发（provider=openai 也由后端删键）", async () => {
    const user = userEvent.setup();
    render(<ModelView />);
    await screen.findByDisplayValue("gpt-5-codex");

    await user.clear(screen.getByPlaceholderText("gpt-5-codex"));
    await user.type(screen.getByPlaceholderText("gpt-5-codex"), "gpt-6");
    await user.click(screen.getAllByRole("button", { name: "Apply" })[0]);

    expect(cmd.modelApply).toHaveBeenCalledWith("gpt-6", "deepseek", "low");
  });

  it("预设 Apply：写入参数来自预设而非当前表单", async () => {
    const user = userEvent.setup();
    render(<ModelView />);
    await screen.findByText("Scout");

    await user.click(screen.getAllByRole("button", { name: "Apply" })[1]);

    expect(cmd.modelApply).toHaveBeenCalledWith("gpt-5.6-luna", "", "low");
  });
});

describe("预设保存与删除", () => {
  it("存预设：只填名字，模型三字段取当前表单值", async () => {
    const user = userEvent.setup();
    render(<ModelView />);
    await screen.findByDisplayValue("gpt-5-codex");

    await user.click(screen.getByRole("button", { name: "Save current as preset" }));
    await user.type(screen.getByPlaceholderText("e.g. Fast scout"), "Main");
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
    render(<ModelView />);
    await screen.findByText("Scout");

    await user.click(screen.getAllByRole("button", { name: "Delete" })[1]);

    expect(askMock).toHaveBeenCalledTimes(1);
    expect(cmd.modelPresetDelete).toHaveBeenCalledWith("p1");
  });
});

describe("供应商保存与删除", () => {
  it("新增供应商：认证选环境变量时只下发 envKey", async () => {
    const user = userEvent.setup();
    render(<ModelView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getByRole("button", { name: "Add Provider" }));
    await user.type(screen.getByPlaceholderText("deepseek"), "kimi");
    await user.type(screen.getByPlaceholderText("DeepSeek"), "Kimi");
    await user.type(screen.getByPlaceholderText("https://api.deepseek.com/v1"), "https://api.kimi.com");
    await user.type(screen.getByPlaceholderText("DEEPSEEK_API_KEY"), "KIMI_API_KEY");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(cmd.modelProviderSave).toHaveBeenCalledWith({
      id: "kimi",
      name: "Kimi",
      baseUrl: "https://api.kimi.com",
      envKey: "KIMI_API_KEY",
      bearerToken: "",
    });
  });

  it("删除供应商：确认流通过后按 id 删除", async () => {
    const user = userEvent.setup();
    render(<ModelView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getAllByRole("button", { name: "Delete" })[0]);

    expect(cmd.modelProviderDelete).toHaveBeenCalledWith("deepseek");
  });

  it("删除供应商：确认弹窗取消则不删除", async () => {
    const user = userEvent.setup();
    askMock.mockResolvedValue(false);
    render(<ModelView />);
    await screen.findByText("DeepSeek");

    await user.click(screen.getAllByRole("button", { name: "Delete" })[0]);

    expect(cmd.modelProviderDelete).not.toHaveBeenCalled();
  });
});
