// service：taskboard/injector 进程启停、状态轮询渲染、Skill 安装、设置页路径浏览

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import { toast, onConfigChange, readConfigFromUI } from "./core";
import { toggleSettings, switchSection } from "./nav";

export interface ProcessInfo {
  name: string;
  status: "stopped" | "starting" | "running" | "stopping" | "failed";
  pid: number | null;
  message: string;
}

type ServiceKey = "taskboard" | "injector";
type ServiceIndicatorState = "stopped" | "starting" | "running" | "failed";

// ============ 使用内置 Taskboard ============
export async function useBundledTaskboard(): Promise<void> {
  try {
    const path = await invoke<string | null>("get_bundled_taskboard_path");
    if (path) {
      (document.getElementById("cfg-path") as HTMLInputElement).value = path;
      onConfigChange();
      toast(t("Using bundled Taskboard path"), "success");
    } else {
      toast(t("Bundled Taskboard not found"), "error");
    }
  } catch (e) {
    toast(t("Failed to get bundled path: {{error}}", { error: String(e) }), "error");
  }
}

// ============ 文件浏览 ============
export async function browsePath(): Promise<void> {
  const selected = await openDialog({ directory: true, multiple: false });
  if (selected) {
    (document.getElementById("cfg-path") as HTMLInputElement).value = selected as string;
    onConfigChange();
  }
}

export async function browseNode(): Promise<void> {
  const selected = await openDialog({
    directory: false,
    multiple: false,
    filters: [{ name: "Node", extensions: ["*"] }],
  });
  if (selected) {
    (document.getElementById("cfg-node") as HTMLInputElement).value = selected as string;
    onConfigChange();
  }
}

export async function browseCodex(): Promise<void> {
  // macOS 选 .app 包（目录）；Windows 选 .exe 文件，目录选择器选不到 exe
  const isWindows = navigator.userAgent.includes("Windows");
  const selected = await openDialog(
    isWindows
      ? { directory: false, multiple: false, filters: [{ name: "Codex", extensions: ["exe"] }] }
      : { directory: true, multiple: false }
  );
  if (selected) {
    (document.getElementById("cfg-codex") as HTMLInputElement).value = selected as string;
    onConfigChange();
  }
}

// ============ 启动/停止 ============
// 后端错误前缀：Codex 已运行但未开 CDP（仅 Windows 发出），其余错误照常抛出
const CODEX_NO_CDP_MARK = "CODEX_RUNNING_NO_CDP|";

// 命中标记时弹窗询问：确认则关闭当前 Codex 并重试启动，取消则返回 false 由调用方终止流程
async function startWithCodexRestart(fn: () => Promise<unknown>): Promise<boolean> {
  try {
    await fn();
    return true;
  } catch (e) {
    if (!String(e).includes(CODEX_NO_CDP_MARK)) throw e;
    if (!(await confirmRestartCodex())) return false;
    setRestartLoading(true);
    try {
      await invoke("quit_codex");
      await fn();
    } finally {
      setRestartLoading(false);
    }
    return true;
  }
}

// 重启 Codex 过渡层显隐（关闭当前实例到重新拉起可能数秒，避免无反馈误以为是卡死）
function setRestartLoading(on: boolean): void {
  document.getElementById("restart-loading-overlay")?.classList.toggle("hidden", !on);
}

// 自定义确认弹窗（替代原生 ask，外观随应用主题）。确认/取消各绑定一次、用完即解绑
function confirmRestartCodex(): Promise<boolean> {
  return new Promise((resolve) => {
    const modal = document.getElementById("codex-restart-modal")!;
    const confirmBtn = document.getElementById("codex-restart-confirm")!;
    const cancelBtn = document.getElementById("codex-restart-cancel")!;
    const cleanup = (yes: boolean) => {
      modal.classList.add("hidden");
      confirmBtn.removeEventListener("click", onConfirm);
      cancelBtn.removeEventListener("click", onCancel);
      resolve(yes);
    };
    const onConfirm = () => cleanup(true);
    const onCancel = () => cleanup(false);
    confirmBtn.addEventListener("click", onConfirm);
    cancelBtn.addEventListener("click", onCancel);
    modal.classList.remove("hidden");
    confirmBtn.focus();
  });
}

export async function startAll(): Promise<void> {
  const cfg = readConfigFromUI();
  if (!cfg.taskboard_path) {
    toast(t("Please configure the Taskboard path in Settings first"), "error");
    const settingsView = document.getElementById("settings-view")!;
    if (settingsView.classList.contains("hidden")) {
      toggleSettings();
    }
    switchSection("general");
    return;
  }

  const btn = document.getElementById("btn-start-all")! as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = t("Starting...");

  try {
    await invoke("update_settings", { config: cfg });
    if (!(await startWithCodexRestart(() => invoke("start_all", { config: cfg })))) {
      toast(t("Launch cancelled"), "info");
      return;
    }
    toast(t("All services started"), "success");
    await refreshStatus();
  } catch (e) {
    toast(t("Launch failed: {{error}}", { error: String(e) }), "error");
  } finally {
    btn.disabled = false;
    btn.textContent = t("Start All");
  }
}

export async function stopAll(): Promise<void> {
  const btn = document.getElementById("btn-stop-all")! as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = t("Stopping...");

  try {
    await invoke("stop_all");
    toast(t("All services stopped"), "info");
    await refreshStatus();
  } catch (e) {
    toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
  } finally {
    btn.disabled = false;
    btn.textContent = t("Stop All");
  }
}

export async function startTaskboard(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    await invoke("start_taskboard", { config: cfg });
    toast(t("Taskboard server started"), "success");
    await refreshStatus();
  } catch (e) {
    toast(t("Launch failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function stopTaskboard(): Promise<void> {
  try {
    await invoke("stop_taskboard");
    toast(t("Taskboard server stopped"), "info");
    await refreshStatus();
  } catch (e) {
    toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function startInjector(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    if (!(await startWithCodexRestart(() => invoke("start_injector", { config: cfg })))) {
      toast(t("Launch cancelled"), "info");
      return;
    }
    toast(t("Codex injector started"), "success");
    await refreshStatus();
  } catch (e) {
    toast(t("Launch failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function stopInjector(): Promise<void> {
  try {
    await invoke("stop_injector");
    toast(t("Codex injector stopped"), "info");
    await refreshStatus();
  } catch (e) {
    toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function openTaskboard(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    await invoke("open_taskboard", { config: cfg });
  } catch (e) {
    toast(t("Open failed: {{error}}", { error: String(e) }), "error");
  }
}

// ============ Skill 安装 ============
interface SkillStatus {
  state: "installed" | "not-installed" | "mismatch";
  detail: string;
  targetPath: string;
}

export async function refreshSkillStatus(): Promise<void> {
  const badge = document.getElementById("skill-status-badge")!;
  const text = document.getElementById("skill-status-text")!;
  const detail = document.getElementById("skill-status-detail")!;
  try {
    const cfg = readConfigFromUI();
    const status = await invoke<SkillStatus>("check_skill_status", {
      taskboardPath: cfg.taskboard_path,
    });
    badge.className = `status-badge ${
      status.state === "installed" ? "running" : status.state === "mismatch" ? "starting" : "stopped"
    }`;
    text.textContent =
      status.state === "installed" ? t("Installed") : status.state === "mismatch" ? t("Installation mismatch") : t("Not installed");
    detail.textContent = status.detail;
  } catch (e) {
    badge.className = "status-badge failed";
    text.textContent = t("Detection failed");
    detail.textContent = String(e);
  }
}

export async function installSkill(): Promise<void> {
  const cfg = readConfigFromUI();
  if (!cfg.taskboard_path) {
    toast(t("Please configure the Taskboard path first"), "error");
    return;
  }
  try {
    const result = await invoke<string>("install_skill", { taskboardPath: cfg.taskboard_path });
    document.getElementById("skill-result")!.textContent = result;
    toast(t("Skill installed successfully"), "success");
  } catch (e) {
    document.getElementById("skill-result")!.textContent = t("Failed: {{error}}", { error: String(e) });
    toast(t("Installation failed: {{error}}", { error: String(e) }), "error");
  }
  await refreshSkillStatus();
}

// ============ 状态更新 ============
// 图标为模块内静态常量，非外部输入
const SERVICE_INDICATOR_SYMBOLS: Record<ServiceIndicatorState, string> = {
  running: `<svg width="56" height="56" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 4 4L19 6" /></svg>`,
  stopped: `<svg width="56" height="56" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 18 18 6M6 6l12 12" /></svg>`,
  starting: `<svg width="56" height="56" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M8.5 5.5c-.8-.5-1.9 0-1.9 1v11c0 1 1.1 1.6 1.9 1.1l8.5-5.5c.8-.5.8-1.7 0-2.2L8.5 5.5z" /></svg>`,
  failed: `<svg width="56" height="56" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 18 18 6M6 6l12 12" /></svg>`,
};

const serviceStatuses: Record<ServiceKey, ProcessInfo["status"]> = {
  taskboard: "stopped",
  injector: "stopped",
};

export async function refreshStatus(): Promise<void> {
  try {
    const statuses = await invoke<ProcessInfo[]>("get_status");
    for (const s of statuses) {
      updateStatusUI(s);
    }
    updateGlobalButtons(statuses);
  } catch {
    // 忽略轮询错误
  }
}

export function updateStatusUI(info: ProcessInfo): void {
  const isTaskboard = info.name === "taskboard-server";
  const prefix = isTaskboard ? "taskboard" : "injector";
  serviceStatuses[prefix] = info.status;

  const badge = document.getElementById(`badge-${prefix}`)!;
  const msg = document.getElementById(`msg-${prefix}`)!;
  const statusText = document.getElementById(`badge-${prefix}-text`)!;

  const statusMap: Record<string, { text: string; cls: string }> = {
    running: { text: t("Running"), cls: "running" },
    stopped: { text: t("Stopped"), cls: "stopped" },
    starting: { text: t("Starting"), cls: "starting" },
    stopping: { text: t("Stopping"), cls: "stopping" },
    failed: { text: t("Failed"), cls: "failed" },
  };

  const s = statusMap[info.status] || statusMap.stopped;
  badge.className = `status-badge ${s.cls}`;
  statusText.textContent = s.text;
  msg.textContent = info.message || "-";

  const startBtn = document.getElementById(`btn-start-${isTaskboard ? "tb" : "inj"}`)! as HTMLButtonElement;
  const stopBtn = document.getElementById(`btn-stop-${isTaskboard ? "tb" : "inj"}`)! as HTMLButtonElement;

  startBtn.disabled = info.status === "running" || info.status === "starting";
  stopBtn.disabled = info.status !== "running";

  if (isTaskboard) {
    const openBtn = document.getElementById("btn-open-tb")! as HTMLButtonElement;
    openBtn.disabled = info.status !== "running";
  }

  updateServiceStatusIndicator();
}

function updateServiceStatusIndicator(): void {
  const statuses = Object.values(serviceStatuses);
  const hasTransition = statuses.some((status) => status === "starting" || status === "stopping");
  const hasRunning = statuses.some((status) => status === "running");
  let state: ServiceIndicatorState;
  let stateText: string;
  if (statuses.includes("failed")) {
    state = "failed";
    stateText = "Service issue";
  } else if (hasTransition) {
    state = "starting";
    stateText = hasRunning ? "Partially running" : "Services starting";
  } else if (statuses.every((status) => status === "running")) {
    state = "running";
    stateText = "All services running";
  } else if (hasRunning) {
    state = "starting";
    stateText = "Partially running";
  } else {
    state = "stopped";
    stateText = "Services stopped";
  }

  const icon = document.getElementById("service-status-indicator-icon")!;
  const symbol = document.getElementById("service-status-indicator-symbol")!;
  const text = document.getElementById("service-status-indicator-text")!;
  const nextIconClass = `status-indicator-icon ${state}`;
  const nextTextClass = `status-indicator-text ${state}`;
  const nextText = t(stateText);
  if (icon.className !== nextIconClass) icon.className = nextIconClass;
  if (symbol.dataset.state !== state) {
    symbol.innerHTML = SERVICE_INDICATOR_SYMBOLS[state];
    symbol.dataset.state = state;
  }
  if (text.className !== nextTextClass) text.className = nextTextClass;
  if (text.textContent !== nextText) text.textContent = nextText;
}

function updateGlobalButtons(statuses: ProcessInfo[]): void {
  const anyRunning = statuses.some((s) => s.status === "running" || s.status === "starting");
  const allStopped = statuses.every((s) => s.status === "stopped" || s.status === "failed");

  (document.getElementById("btn-start-all")! as HTMLButtonElement).disabled = anyRunning;
  (document.getElementById("btn-stop-all")! as HTMLButtonElement).disabled = allStopped;
}
