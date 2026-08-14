// dsh：DeepSeek Harness 远程访问（Tailscale）——状态渲染、时间轴安装进度、开机自启
// 架构参照教程《用 Tailscale 远程访问 DeepSeek Harness (dsh) 完整教程》：
//   https://<host>.ts.net (tailscale serve 443)
//     → 127.0.0.1:3898 (loopback 反代: Host→127.0.0.1, 删 Origin)
//     → 127.0.0.1:3899 (dsh web)
// 安装进度以时间轴展示；失败节点内嵌「问题 + 解决方案」

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { t } from "./i18n";
import { toast, escapeHtml } from "./core";

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

// 时间轴步骤顺序（与 Rust dsh_setup 的 index 一一对应）
const STEP_IDS = ["node", "install", "start", "tailscale", "magicdns", "proxy", "serve", "verify"] as const;

// 步骤标题（key 即 i18n key）
const STEP_TITLES: Record<string, string> = {
  node: "Check Node.js & npm",
  install: "Install DeepSeek Harness (dsh)",
  start: "Start dsh Web",
  tailscale: "Check Tailscale",
  magicdns: "Enable MagicDNS",
  proxy: "Start loopback proxy",
  serve: "Configure Tailscale serve",
  verify: "Verify remote access",
};

let dshStatus: DshStatus | null = null;
let dshBusy = false;
// 是否跑过一键安装流程：跑过则时间轴以事件流为准，否则用检测结果渲染就绪视图
let hasRunSetup = false;
let timelineSteps: DshStepEvent[] = [];

// ============ 渲染 ============

function statusText(s: DshStatus): string {
  if (!s.nodeAvailable) return t("Node.js not detected");
  if (!s.dshInstalled) return t("DeepSeek Harness not installed");
  if (!s.dshRunning) return t("dsh web not running");
  if (!s.tailscaleInstalled || !s.tailscaleOnline) return t("Tailscale not ready");
  if (!s.magicDnsEnabled) return t("MagicDNS not enabled");
  if (!s.proxyRunning) return t("Loopback proxy not running");
  if (!s.serveConfigured) return t("Tailscale serve not configured");
  return t("Remote access ready");
}

function markerFor(state: string): string {
  switch (state) {
    case "done":
      return "✓";
    case "failed":
      return "✕";
    case "running":
      return `<span class="timeline-spinner"></span>`;
    case "skipped":
      return "–";
    default:
      return "○";
  }
}

function renderTimeline(): void {
  const container = document.getElementById("dsh-timeline-nodes");
  if (!container) return;
  container.innerHTML = timelineSteps
    .map((step) => {
      const title = t(STEP_TITLES[step.id] ?? step.id);
      const detail = step.detail ? `<div class="timeline-detail">${escapeHtml(step.detail)}</div>` : "";
      const issue =
        step.state === "failed" && (step.problem || step.solution)
          ? `<div class="timeline-issue">
               ${step.problem ? `<div class="timeline-problem">${escapeHtml(step.problem)}</div>` : ""}
               ${step.solution ? `<div class="timeline-solution">${escapeHtml(step.solution)}</div>` : ""}
             </div>`
          : "";
      return `<div class="timeline-node" data-state="${step.state}">
        <div class="timeline-marker">${markerFor(step.state)}</div>
        <div class="timeline-content">
          <div class="timeline-title">${escapeHtml(title)}</div>${detail}${issue}
        </div>
      </div>`;
    })
    .join("");
}

// 由检测结果推导「就绪时间轴」：已满足的步骤标 done，其余 pending
function timelineFromStatus(s: DshStatus): DshStepEvent[] {
  const allReady =
    s.nodeAvailable && s.dshInstalled && s.dshRunning && s.tailscaleOnline && s.magicDnsEnabled && s.proxyRunning && s.serveConfigured;
  const done = (ok: boolean): DshStepEvent["state"] => (ok ? "done" : "pending");
  return [
    { index: 0, id: "node", state: done(s.nodeAvailable), detail: null, problem: null, solution: null },
    { index: 1, id: "install", state: done(s.dshInstalled), detail: null, problem: null, solution: null },
    { index: 2, id: "start", state: done(s.dshRunning), detail: null, problem: null, solution: null },
    { index: 3, id: "tailscale", state: done(s.tailscaleInstalled && s.tailscaleOnline), detail: null, problem: null, solution: null },
    { index: 4, id: "magicdns", state: done(s.magicDnsEnabled), detail: null, problem: null, solution: null },
    { index: 5, id: "proxy", state: done(s.proxyRunning), detail: null, problem: null, solution: null },
    { index: 6, id: "serve", state: done(s.serveConfigured), detail: null, problem: null, solution: null },
    { index: 7, id: "verify", state: done(allReady), detail: null, problem: null, solution: null },
  ];
}

export function renderDsh(): void {
  const statusEl = document.getElementById("dsh-status");
  const versionEl = document.getElementById("dsh-version");
  const pill = document.getElementById("dsh-url-pill");
  const startBtn = document.getElementById("btn-dsh-start") as HTMLButtonElement | null;
  const stopBtn = document.getElementById("btn-dsh-stop") as HTMLButtonElement | null;
  const openBtn = document.getElementById("btn-dsh-open") as HTMLButtonElement | null;
  const updateBtn = document.getElementById("btn-dsh-update") as HTMLButtonElement | null;
  const autostartToggle = document.getElementById("toggle-dsh-autostart") as HTMLInputElement | null;
  if (!statusEl) return;

  if (dshBusy) {
    statusEl.textContent = t("Working…");
  } else if (dshStatus) {
    statusEl.textContent = statusText(dshStatus);
  } else {
    statusEl.textContent = t("Detecting…");
  }

  // 已安装时显示当前版本胶囊
  if (versionEl) {
    if (dshStatus?.dshVersion) {
      versionEl.classList.remove("hidden");
      versionEl.textContent = dshStatus.dshVersion;
    } else {
      versionEl.classList.add("hidden");
    }
  }

  if (pill) {
    if (dshStatus?.url && !dshBusy) {
      pill.classList.remove("hidden");
      pill.textContent = dshStatus.url;
    } else {
      pill.classList.add("hidden");
    }
  }

  if (startBtn) {
    startBtn.disabled = dshBusy;
    startBtn.textContent = dshBusy ? t("Working…") : t("One-click remote access");
  }
  if (stopBtn) {
    stopBtn.disabled = dshBusy || !dshStatus || (!dshStatus.dshRunning && !dshStatus.proxyRunning);
  }
  if (openBtn) {
    openBtn.disabled = dshBusy || !dshStatus?.url;
  }
  // 更新按钮：仅在有新版可更时显示
  if (updateBtn) {
    const latest = dshStatus?.latestVersion;
    const hasUpdate = !!latest;
    updateBtn.classList.toggle("hidden", !hasUpdate);
    if (hasUpdate) {
      updateBtn.textContent = t("Update to {{version}}", { version: latest });
    }
    updateBtn.disabled = dshBusy;
  }
  if (autostartToggle && dshStatus) {
    autostartToggle.checked = dshStatus.autostartEnabled;
  }

  renderTimeline();
}

// ============ 检测 ============

export async function refreshDshStatus(): Promise<void> {
  try {
    dshStatus = await invoke<DshStatus>("dsh_detect");
    if (!hasRunSetup) {
      timelineSteps = timelineFromStatus(dshStatus);
    }
  } catch (e) {
    toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
  }
  renderDsh();
}

// ============ 一键启动（时间轴） ============

export async function startDshRemote(): Promise<void> {
  if (dshBusy) return;
  dshBusy = true;
  hasRunSetup = true;
  // 初始化为全 pending，随后由后端 dsh-step 事件逐步推进
  timelineSteps = STEP_IDS.map((id, index) => ({
    index,
    id,
    state: "pending" as const,
    detail: null,
    problem: null,
    solution: null,
  }));
  renderDsh();
  try {
    await invoke("dsh_setup");
    toast(t("Remote access is ready"), "success");
  } catch {
    // 失败详情（问题 + 解决方案）已由 dsh-step 事件渲染在时间轴节点上
  } finally {
    dshBusy = false;
    await refreshDshStatus();
  }
}

export async function stopDshRemote(): Promise<void> {
  try {
    await invoke("dsh_stop");
    toast(t("dsh remote access services stopped"), "info");
  } catch (e) {
    toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
  }
  await refreshDshStatus();
}

export async function openDshRemote(): Promise<void> {
  if (!dshStatus?.url) {
    toast(t("Remote URL not available yet; run the one-click setup first"), "error");
    return;
  }
  try {
    await openUrl(dshStatus.url);
  } catch (e) {
    toast(t("Failed to open: {{error}}", { error: String(e) }), "error");
  }
}

// ============ 更新 ============

export async function updateDsh(): Promise<void> {
  if (dshBusy) return;
  dshBusy = true;
  renderDsh();
  try {
    const version = await invoke<string>("dsh_update");
    toast(t("dsh updated to {{version}}", { version }), "success");
  } catch (e) {
    toast(t("dsh update failed: {{error}}", { error: String(e) }), "error");
  } finally {
    dshBusy = false;
    await refreshDshStatus();
  }
}

// ============ 开机自启 ============

export async function toggleDshAutostart(): Promise<void> {
  const el = document.getElementById("toggle-dsh-autostart") as HTMLInputElement;
  const next = el.checked;
  try {
    await invoke("dsh_set_autostart", { enabled: next });
    if (dshStatus) dshStatus.autostartEnabled = next;
    toast(next ? t("Auto-start enabled") : t("Auto-start disabled"), "success");
  } catch (e) {
    el.checked = !next;
    toast(t("Failed to change auto-start: {{error}}", { error: String(e) }), "error");
  }
}

// ============ 事件 ============

function handleDshStep(step: DshStepEvent): void {
  const i = timelineSteps.findIndex((s) => s.index === step.index);
  if (i >= 0) {
    timelineSteps[i] = step;
  } else {
    timelineSteps.push(step);
    timelineSteps.sort((a, b) => a.index - b.index);
  }
  renderDsh();
}

export async function bindDshEvents(): Promise<void> {
  await listen<DshStepEvent>("dsh-step", (event) => {
    handleDshStep(event.payload);
  });
}
