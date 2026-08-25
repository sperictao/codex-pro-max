// 看守域组件/操作测试（ADR 0010：测试范围限定看守域——四 apply_mode 与锁定状态机是回归风险最集中处）
// commands 与 plugin-dialog 全量 mock；断言落点：invoke 参数、toast 内容、DOM 状态

import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import type { GuardFileView, GuardParamView, GuardView as GuardViewData } from "@/shared/types";
import { GuardView } from "./GuardView";
import { GuardSettingsSection } from "./GuardSettingsSection";
import * as ops from "./ops";

vi.mock("@/shared/commands", () => ({
  guardGetView: vi.fn(),
  guardGetFiles: vi.fn(),
  guardSetValue: vi.fn(),
  guardApply: vi.fn(),
  guardSetApplied: vi.fn(),
  guardSetLocked: vi.fn(),
  guardSetEnabled: vi.fn(),
  guardRemoveCustomParam: vi.fn(),
  guardAddCustomParam: vi.fn(),
  guardDetectFile: vi.fn(),
  guardUpdateFile: vi.fn(),
  guardAddFile: vi.fn(),
  guardRemoveFile: vi.fn(),
  guardGetSchemaFilePath: vi.fn(),
  guardRelativizePickedPath: vi.fn(),
}));

const askMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: (...args: unknown[]) => askMock(...args),
  open: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-shell", () => ({ open: vi.fn() }));

function makeParam(over: Partial<GuardParamView>): GuardParamView {
  return {
    id: "p1",
    label: "Param One",
    description: "",
    applyMode: "toml_key",
    valueType: "bool",
    path: "features.p1",
    default: true,
    value: false,
    applied: true,
    locked: false,
    actual: "false",
    status: "match",
    error: null,
    lastChecked: null,
    lastRestored: null,
    custom: false,
    ...over,
  };
}

function makeView(params: GuardParamView[]): GuardViewData {
  return {
    enabled: true,
    groups: [
      { id: "g1", name: "config.toml", file: "config.toml", format: "toml", builtin: true, error: null, params },
    ],
  };
}

function makeFile(over: Partial<GuardFileView>): GuardFileView {
  return {
    id: "f1",
    name: "config.toml",
    file: "config.toml",
    format: "toml",
    builtin: true,
    detection: { path: "config.toml", at: 1700000000 },
    ...over,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState(initialState, true);
  useAppStore.setState({ activeView: "guard" });
});

describe("看守视图渲染", () => {
  it("渲染分组与参数卡：状态徽标、Current 行、锁定参数编辑器禁用且显示时间行", async () => {
    const view = makeView([
      makeParam({ id: "p1" }),
      makeParam({ id: "p2", label: "Locked One", locked: true, lastChecked: 1700000000, lastRestored: null }),
    ]);
    vi.mocked(cmd.guardGetView).mockResolvedValue(view);
    vi.mocked(cmd.guardGetFiles).mockResolvedValue([]);

    render(<GuardView />);

    await screen.findByText("Locked One");
    expect(screen.getAllByText("Match")).toHaveLength(2);
    expect(screen.getByText("config.toml")).toBeInTheDocument();
    // 锁定参数：编辑器禁用 + 时间行 + Unlock 按钮
    const lockedCard = screen.getByText("Locked One").closest(".guard-param-card")!;
    expect(lockedCard.querySelector("input[data-guard-id='p2']")).toBeDisabled();
    expect(lockedCard.textContent).toContain("Last checked");
    expect(screen.getByRole("button", { name: "Unlock" })).toBeInTheDocument();
  });

  it("未启用参数的 Lock 按钮禁用（须先启用才能锁）", async () => {
    const view = makeView([makeParam({ id: "p1", applied: false })]);
    vi.mocked(cmd.guardGetView).mockResolvedValue(view);
    vi.mocked(cmd.guardGetFiles).mockResolvedValue([]);

    render(<GuardView />);

    await screen.findAllByText("Param One");
    expect(screen.getByRole("button", { name: "Lock" })).toBeDisabled();
  });
});

// 等参数卡渲染完成后按 data-guard-id 取编辑器（label 与嵌套帮助文本会多重命中，故不用 findByText）
async function paramInput(id: string): Promise<HTMLElement> {
  await waitFor(() => expect(document.querySelector(`[data-guard-id='${id}']`)).not.toBeNull());
  return document.querySelector<HTMLElement>(`[data-guard-id='${id}']`)!;
}

describe("参数操作", () => {
  it("bool 开关：取反当前值落盘并强制刷新", async () => {
    vi.mocked(cmd.guardGetView).mockResolvedValue(makeView([makeParam({ id: "p1", value: false })]));
    vi.mocked(cmd.guardGetFiles).mockResolvedValue([]);
    vi.mocked(cmd.guardSetValue).mockResolvedValue(undefined);

    render(<GuardView />);
    await userEvent.click(await paramInput("p1"));

    expect(cmd.guardSetValue).toHaveBeenCalledWith("p1", true);
  });

  it("int 输入非整数：错误 toast 且不写值", async () => {
    const view = makeView([makeParam({ id: "p1", valueType: "int", value: 42, default: 60 })]);
    vi.mocked(cmd.guardGetView).mockResolvedValue(view);
    vi.mocked(cmd.guardGetFiles).mockResolvedValue([]);

    render(<GuardView />);
    const input = (await paramInput("p1")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input);

    await waitFor(() =>
      expect(useAppStore.getState().toasts.some((t) => t.message === "Please enter an integer")).toBe(true),
    );
    expect(cmd.guardSetValue).not.toHaveBeenCalled();
  });

  it("启用开关：已启用 → guardSetApplied(false)；未启用 → guardApply", async () => {
    vi.mocked(cmd.guardGetView).mockResolvedValue(makeView([makeParam({ id: "p1", applied: true })]));
    vi.mocked(cmd.guardSetApplied).mockResolvedValue(undefined);
    await ops.toggleApplied("p1");
    expect(cmd.guardSetApplied).toHaveBeenCalledWith("p1", false);
    expect(cmd.guardApply).not.toHaveBeenCalled();

    vi.clearAllMocks();
    vi.mocked(cmd.guardGetView).mockResolvedValue(makeView([makeParam({ id: "p1", applied: false })]));
    vi.mocked(cmd.guardApply).mockResolvedValue(undefined);
    await ops.toggleApplied("p1");
    expect(cmd.guardApply).toHaveBeenCalledWith("p1");
  });

  it("锁定参数不接受 bool 取反（guardState.params 锁定短路）", async () => {
    useAppStore.setState({ guardState: { enabled: true, params: { p1: { locked: true } } } });
    await ops.toggleBool("p1");
    expect(cmd.guardSetValue).not.toHaveBeenCalled();
    expect(cmd.guardGetView).not.toHaveBeenCalled();
  });
});

describe("自定义参数", () => {
  const baseForm = {
    id: "my_param",
    label: "My Param",
    fileId: "f1",
    mode: "toml_key",
    path: "features.my_param",
    valueType: "bool",
    desc: "",
    defaultRaw: "true",
  };

  it("空 ID 校验：错误 toast 且不 invoke", async () => {
    const ok = await ops.addCustom({ ...baseForm, id: "" });
    expect(ok).toBe(false);
    expect(useAppStore.getState().toasts.some((t) => t.message === "Please enter an ID")).toBe(true);
    expect(cmd.guardAddCustomParam).not.toHaveBeenCalled();
  });

  it("int 默认值非法：Add failed toast 且不 invoke", async () => {
    const ok = await ops.addCustom({ ...baseForm, valueType: "int", defaultRaw: "abc" });
    expect(ok).toBe(false);
    expect(
      useAppStore.getState().toasts.some((t) => t.message.includes("Default value must be an integer")),
    ).toBe(true);
    expect(cmd.guardAddCustomParam).not.toHaveBeenCalled();
  });

  it("file_overwrite 固定 text 类型并提交（effectiveType 语义）", async () => {
    vi.mocked(cmd.guardAddCustomParam).mockResolvedValue(undefined);
    vi.mocked(cmd.guardGetView).mockResolvedValue(makeView([]));
    const ok = await ops.addCustom({ ...baseForm, mode: "file_overwrite", path: "", valueType: "bool", defaultRaw: "hello" });
    expect(ok).toBe(true);
    expect(cmd.guardAddCustomParam).toHaveBeenCalledWith(
      expect.objectContaining({ apply_mode: "file_overwrite", value_type: "text", default: "hello", custom: true }),
      "f1",
    );
  });
});

describe("看守文件管理", () => {
  it("文件列表：内置文件 Built-in 禁用 + Detect；自定义文件有 Delete", async () => {
    vi.mocked(cmd.guardGetFiles).mockResolvedValue([
      makeFile({ id: "f1" }),
      makeFile({ id: "f2", name: "extra.toml", file: "extra.toml", builtin: false, detection: null }),
    ]);

    render(<GuardSettingsSection />);

    await screen.findByText("extra.toml");
    expect(screen.getByRole("button", { name: "Built-in" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Detect" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete" })).toBeInTheDocument();
  });

  it("检测路径不一致：原生 ask 确认后更新配置路径", async () => {
    useAppStore.setState({ guardFiles: [makeFile({ id: "f1" })] });
    vi.mocked(cmd.guardDetectFile).mockResolvedValue(
      makeFile({ id: "f1", detection: { path: "real/config.toml", at: 1700000001 } }),
    );
    vi.mocked(cmd.guardUpdateFile).mockResolvedValue(undefined);
    vi.mocked(cmd.guardGetFiles).mockResolvedValue([makeFile({ id: "f1", file: "real/config.toml" })]);
    askMock.mockResolvedValue(true);

    await ops.detectFile("f1");

    expect(askMock).toHaveBeenCalledOnce();
    expect(cmd.guardUpdateFile).toHaveBeenCalledWith("f1", "config.toml", "real/config.toml");
  });

  it("删除文件：ask 取消则不删除", async () => {
    useAppStore.setState({ guardFiles: [makeFile({ id: "f2", builtin: false })] });
    askMock.mockResolvedValue(false);

    await ops.removeFile("f2");

    expect(askMock).toHaveBeenCalledOnce();
    expect(cmd.guardRemoveFile).not.toHaveBeenCalled();
  });
});
