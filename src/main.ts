import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { open as openDialog, ask } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { initI18n, applyDomTranslations, applyLanguage, currentLanguage, t } from "./i18n";
import type { ResolvedLanguage } from "./i18n";
import type { ThemeMode } from "./theme";
import { state } from "./state";
import type { LauncherConfig } from "./state";
import {
  toast,
  escapeHtml,
  fmtTs,
  getStoredTheme,
  applyTheme,
  setTheme,
  setThemeFamily,
  renderThemeFamilyGrid,
  renderLanguageCards,
  readConfigFromUI,
  toggleAutostart,
  openLogDir,
  onConfigChange,
  saveConfig,
  updateModeLabel,
  updateAutoOpenLabel,
  validatePaths,
} from "./core";
import { toggleSettings, showHome, showSkill, showGuard, switchSection } from "./nav";
import "./style.css";

// ============ 类型定义（共享配置类型在 state.ts；各域类型 step 5 随域迁出） ============
interface GuardParamView {
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

interface GuardGroupView {
  id: string;
  name: string;
  file: string;
  format: string;
  builtin: boolean;
  error: string | null;
  params: GuardParamView[];
}

interface GuardFileView {
  id: string;
  name: string;
  file: string;
  format: string;
  builtin: boolean;
  detection: { path: string | null; at: number } | null;
}

interface GuardView {
  enabled: boolean;
  groups: GuardGroupView[];
}

interface ProcessInfo {
  name: string;
  status: "stopped" | "starting" | "running" | "stopping" | "failed";
  pid: number | null;
  message: string;
}

type ServiceKey = "taskboard" | "injector";
type ServiceIndicatorState = "stopped" | "starting" | "running" | "failed";

interface UpdaterConfigHealth {
  configured: boolean;
  message: string;
}

interface UpdaterHelpPaths {
  docsPath: string;
  templatePath: string;
}

interface FastctxStatus {
  installed: boolean;
  version: string | null;
  integrated: boolean;
}

interface FastctxApplyResult {
  selfCheckPassed: boolean;
  selfCheckOutput: string;
}

interface UpdateInfo {
  currentVersion: string;
  availableVersion: string | null;
  hasUpdate: boolean;
  releaseNotes: string | null;
  message: string | null;
}

interface DownloadProgress {
  stage: string;
  version: string;
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number | null;
  attempt: number;
  maxAttempts: number;
}

// ============ 语言管理 ============
// 设置项三选一（system/en/zh-CN）持久化在 LauncherConfig.language；
// Rust 侧经 set_language 重解析并重建托盘，前端经 applyLanguage 重渲染
async function setLanguage(setting: string): Promise<void> {
  state.languageSetting = setting;
  renderLanguageCards();
  try {
    await invoke("update_settings", { config: readConfigFromUI() });
    await invoke("set_language", { setting });
    const resolved = (await invoke<string>("get_resolved_language")) as ResolvedLanguage;
    await applyLanguage(resolved);
    rerenderDynamicText();
  } catch (e) {
    toast(t("Save failed: {{error}}", { error: String(e) }), "error");
  }
}

// 语言切换后重渲染所有动态文本（静态文本由 applyLanguage 扫描 data-i18n）
function rerenderDynamicText(): void {
  updateModeLabel();
  updateAutoOpenLabel();
  renderFastctx();
  renderGuardFiles();
  renderUpdateInfo(pendingUpdateInfo ?? {
    currentVersion: "", availableVersion: null, hasUpdate: false, releaseNotes: null, message: null,
  });
  void refreshStatus();
  void refreshGuardView(true);
  void validatePaths();
  void refreshSkillStatus();
}

// ============ 配置管理（壳侧编排；读写与校验在 core.ts） ============
function fillConfigUI(cfg: LauncherConfig): void {
  (document.getElementById("cfg-path") as HTMLInputElement).value = cfg.taskboard_path;
  (document.getElementById("cfg-node") as HTMLInputElement).value = cfg.node_path;
  (document.getElementById("cfg-codex") as HTMLInputElement).value = cfg.codex_app_path;
  (document.getElementById("cfg-host") as HTMLInputElement).value = cfg.taskboard_host;
  (document.getElementById("cfg-port") as HTMLInputElement).value = String(cfg.taskboard_port);
  (document.getElementById("cfg-cdp") as HTMLInputElement).value = String(cfg.cdp_port);

  const modeToggle = document.getElementById("toggle-mode") as HTMLInputElement;
  modeToggle.checked = cfg.separate_window_mode;
  updateModeLabel();

  const autoOpenToggle = document.getElementById("toggle-auto-open") as HTMLInputElement;
  autoOpenToggle.checked = cfg.auto_open;
  updateAutoOpenLabel();

  (document.getElementById("toggle-tray") as HTMLInputElement).checked = cfg.minimize_to_tray_on_close;

  state.guardState = cfg.codex_guard ?? { enabled: false, params: {} };
  renderGuardToggle();

  state.languageSetting = cfg.language || "system";
  renderLanguageCards();
}

function renderGuardToggle(): void {
  const el = document.getElementById("settings-guard-toggle") as HTMLInputElement | null;
  if (el) el.checked = state.guardState.enabled;
  // 总开关关闭时隐藏顶部「看守」Tab
  const btn = document.getElementById("btn-guard");
  if (btn) btn.classList.toggle("hidden", !state.guardState.enabled);
  // 如果关了总开关且当前在看守页，跳回主页
  if (!state.guardState.enabled) {
    const view = document.getElementById("guard-view");
    if (view && !view.classList.contains("hidden")) {
      showHome();
    }
  }
}

// ============ 使用内置 Taskboard ============
async function useBundledTaskboard(): Promise<void> {
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
async function browsePath(): Promise<void> {
  const selected = await openDialog({ directory: true, multiple: false });
  if (selected) {
    (document.getElementById("cfg-path") as HTMLInputElement).value = selected as string;
    onConfigChange();
  }
}

async function browseNode(): Promise<void> {
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

async function browseCodex(): Promise<void> {
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
async function startAll(): Promise<void> {
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
    await invoke("start_all", { config: cfg });
    toast(t("All services started"), "success");
    await refreshStatus();
  } catch (e) {
    toast(t("Launch failed: {{error}}", { error: String(e) }), "error");
  } finally {
    btn.disabled = false;
    btn.textContent = t("Start All");
  }
}

async function stopAll(): Promise<void> {
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

async function startTaskboard(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    await invoke("start_taskboard", { config: cfg });
    toast(t("Taskboard server started"), "success");
    await refreshStatus();
  } catch (e) {
    toast(t("Launch failed: {{error}}", { error: String(e) }), "error");
  }
}

async function stopTaskboard(): Promise<void> {
  try {
    await invoke("stop_taskboard");
    toast(t("Taskboard server stopped"), "info");
    await refreshStatus();
  } catch (e) {
    toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
  }
}

async function startInjector(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    await invoke("start_injector", { config: cfg });
    toast(t("Codex injector started"), "success");
    await refreshStatus();
  } catch (e) {
    toast(t("Launch failed: {{error}}", { error: String(e) }), "error");
  }
}

async function stopInjector(): Promise<void> {
  try {
    await invoke("stop_injector");
    toast(t("Codex injector stopped"), "info");
    await refreshStatus();
  } catch (e) {
    toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
  }
}

async function openTaskboard(): Promise<void> {
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

async function refreshSkillStatus(): Promise<void> {
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

async function installSkill(): Promise<void> {
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

// ============ Codex 配置看守 ============
async function toggleGuard(): Promise<void> {
  const enabled = !state.guardState.enabled;
  try {
    await invoke("guard_set_enabled", { enabled });
    state.guardState.enabled = enabled;
    renderGuardToggle();
    toast(enabled ? t("Config guard enabled") : t("Config guard disabled"), enabled ? "success" : "info");
  } catch (e) {
    renderGuardToggle();
    toast(t("Toggle failed: {{error}}", { error: String(e) }), "error");
  }
}

// ============ FastCtx 集成 ============
// 接入/摘除委托 fastctx CLI（ADR 0003）；状态以 config.toml 为准实时检测，不持久化开关
let fastctxState: FastctxStatus = { installed: false, version: null, integrated: false };
let fastctxBusy = false;

function renderFastctx(): void {
  (document.getElementById("toggle-fastctx") as HTMLInputElement).checked = fastctxState.integrated;
  const status = document.getElementById("fastctx-status")!;
  const hint = document.getElementById("fastctx-install-hint")!;
  if (fastctxBusy) {
    status.textContent = t("Working…");
  } else if (!fastctxState.installed) {
    status.textContent = t("Not installed");
  } else if (fastctxState.integrated) {
    status.textContent = `${t("Integrated")}${fastctxState.version ? ` · ${fastctxState.version}` : ""}`;
  } else {
    status.textContent = t("Installed{{version}}, not integrated", {
      version: fastctxState.version ? ` (${fastctxState.version})` : "",
    });
  }
  hint.classList.toggle("hidden", fastctxState.installed);
}

async function refreshFastctxStatus(): Promise<void> {
  try {
    fastctxState = await invoke<FastctxStatus>("fastctx_detect");
  } catch (e) {
    toast(t("fastctx detection failed: {{error}}", { error: String(e) }), "error");
  }
  renderFastctx();
}

async function toggleFastctx(): Promise<void> {
  if (fastctxBusy) {
    renderFastctx();
    return;
  }
  if (fastctxState.integrated) {
    const ok = await ask(
      t("Unapply will stop fastctx processes and delete ~/.fastctx managed data (the npm package stays and can be re-integrated anytime). Codex configuration written by fastctx will be removed.\n\nProceed with unapply?"),
      { title: t("Unapply fastctx"), kind: "warning" },
    );
    if (!ok) {
      renderFastctx();
      return;
    }
  }
  fastctxBusy = true;
  renderFastctx();
  try {
    if (!fastctxState.installed) {
      await invoke("fastctx_install");
      toast(t("fastctx installed; integrating…"), "info");
      fastctxState = await invoke<FastctxStatus>("fastctx_detect");
    }
    if (fastctxState.integrated) {
      await invoke("fastctx_unapply");
      toast(t("fastctx unapplied; restart Codex sessions to take full effect"), "info");
    } else {
      const res = await invoke<FastctxApplyResult>("fastctx_apply");
      toast(t("fastctx integrated; restart Codex sessions to activate"), "success");
      if (!res.selfCheckPassed) {
        const line = res.selfCheckOutput.split("\n").find((l) => l.includes("[FAIL]")) ?? res.selfCheckOutput.split("\n")[0] ?? "";
        toast(t("fastctx self-check failed: {{line}} (open the console to troubleshoot)", { line }), "error");
      }
    }
  } catch (e) {
    toast(t("fastctx operation failed: {{error}}", { error: String(e) }), "error");
  } finally {
    fastctxBusy = false;
    await refreshFastctxStatus();
  }
}

async function openFastctxConsole(): Promise<void> {
  if (!fastctxState.installed) {
    toast(t("fastctx not detected; turn on the integration toggle to install it automatically"), "error");
    return;
  }
  try {
    await invoke("fastctx_open_console");
  } catch (e) {
    toast(t("Failed to open console: {{error}}", { error: String(e) }), "error");
  }
}

// ============ 看守视图 ============
let lastGuardJson = "";

async function refreshGuardView(force = false): Promise<void> {
  const viewEl = document.getElementById("guard-view");
  if (!viewEl || viewEl.classList.contains("hidden")) return;
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const json = JSON.stringify(view);
    if (!force && json === lastGuardJson) return;
    // 用户正在输入时不重渲染，避免抢走焦点/清空草稿
    const ae = document.activeElement;
    if (!force && ae && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA")
        && viewEl.contains(ae)) return;
    lastGuardJson = json;
    renderGuardView(view);
    renderGuardToggle();
  } catch {
    // 轮询错误忽略
  }
}

const LOCK_SVG = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>`;
const UNLOCK_SVG = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 9.9-1"/></svg>`;

function renderGuardView(view: GuardView): void {
  const container = document.getElementById("guard-groups")!;
  const statusMap: Record<string, { text: string; cls: string }> = {
    match: { text: t("Match"), cls: "running" },
    drift: { text: t("Drift"), cls: "failed" },
    missing: { text: t("Missing"), cls: "starting" },
    error: { text: t("Error"), cls: "failed" },
  };
  // 渲染内容来自本地 schema/配置文件，非远程输入；动态文本一律 escapeHtml
  container.innerHTML = view.groups.map((g) => {
    const params = g.params.map((p) => {
      const s = statusMap[p.status] ?? statusMap.error;
      let editor = "";
      const dis = p.locked ? "disabled" : "";
      if (p.valueType === "bool") {
        editor = `<div class="flex items-center gap-2">
          <input type="checkbox" class="relative h-5 w-9 shrink-0 cursor-pointer appearance-none rounded-full bg-input transition-colors outline-none checked:bg-primary focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 before:absolute before:left-0.5 before:top-0.5 before:h-4 before:w-4 before:rounded-full before:bg-background before:shadow-sm before:transition-transform checked:before:translate-x-4" data-change-action="guardToggleBool" data-id="${p.id}"
                 ${p.value === true ? "checked" : ""} ${p.locked ? "disabled" : ""} />
          <span class="text-xs opacity-70">${p.value === true ? "true" : "false"} ${t("(recommended {{default}})", { default: String(p.default) })}</span>
        </div>`;
      } else if (p.valueType === "int" || p.valueType === "string") {
        const t = p.valueType === "int" ? "number" : "text";
        editor = `<input type="${t}" class="h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-xs outline-none transition-colors placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 font-mono" ${dis}
               value="${escapeHtml(String(p.value ?? ""))}" data-guard-id="${p.id}"
               data-change-action="guardSetValue" data-id="${p.id}" />`;
      } else if (p.valueType === "text") {
        editor = `<textarea class="guard-textarea" ${dis} data-guard-id="${p.id}"
               data-change-action="guardSetValue" data-id="${p.id}">${escapeHtml(String(p.value ?? ""))}</textarea>`;
      } else {
        editor = `<span class="text-xs opacity-60">${t("No editable value; applying performs \"{{action}}\"", { action: t(p.applyMode === "toml_absent" ? "delete" : "write") })}</span>`;
      }
      const meta = p.locked
        ? `<div class="mt-1 text-xs opacity-50">${t("Last checked {{checked}} | Last auto-restored {{restored}}", { checked: fmtTs(p.lastChecked), restored: fmtTs(p.lastRestored) })}</div>`
        : "";
      return `<div class="guard-param-card rounded-lg border border-border bg-card text-card-foreground p-3">
        <div class="flex items-center justify-between">
          <span class="text-sm font-medium">${escapeHtml(p.label)}${p.description || p.path ? ` <span class="guard-param-help" tabindex="0">?<span class="guard-param-desc">${p.description ? `<span>${escapeHtml(p.description)}</span>` : ""}${p.path ? `<span class="guard-param-desc-path">${escapeHtml(p.path)}</span>` : ""}</span></span>` : ""}</span>
          <span class="status-badge ${s.cls}"><span class="dot"></span><span>${s.text}</span></span>
        </div>
        <div class="mt-1 flex items-start justify-between gap-2">
          <div class="min-w-0 flex-1">
            <div class="guard-param-actual font-mono text-xs ${p.status === "match" ? "ok" : "bad"}">
              ${t("Current: ")}${escapeHtml(p.actual ?? p.error ?? t("Unknown"))}
            </div>
            <div class="mt-2">${editor}</div>
            ${meta}
          </div>
          <span class="guard-param-actions flex w-[30%] shrink-0 flex-row flex-wrap items-center justify-end gap-1 self-center">
            <input type="checkbox" class="text-switch" data-change-action="guardToggleApplied" data-id="${p.id}" data-state-text="${p.applied ? t("Enabled") : t("Disabled")}"
                   ${p.applied ? "checked" : ""} ${p.locked ? "disabled" : ""} title="${p.applied ? t("Disable") : t("Enable")}" aria-label="${p.applied ? t("Disable") : t("Enable")}" />
            ${p.locked
              ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-1 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="guardSetLocked" data-id="${p.id}" data-locked="false">${LOCK_SVG}${t("Unlock")}</button>`
              : `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-1 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" ${p.applied ? "" : "disabled"}
                    data-action="guardSetLocked" data-id="${p.id}" data-locked="true">${UNLOCK_SVG}${t("Lock")}</button>`}
            ${p.custom ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-destructive/50 bg-background text-destructive hover:bg-destructive/10 h-6 px-2 text-xs" data-action="guardRemoveCustom" data-id="${p.id}" title="${t("Delete custom parameter")}">${t("Delete")}</button>` : ""}
          </span>
        </div>
      </div>`;
    }).join("");
    const addBtn = `<div class="mt-2">
      <button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="openGuardAddFormFor" data-id="${g.id}">${t("+ Add Parameter")}</button>
    </div>`;
    return `<div class="rounded-xl border border-border bg-card text-card-foreground p-4" data-group-id="${g.id}">
      <div class="text-sm font-semibold">${escapeHtml(g.name)}</div>
      <div class="mb-2 font-mono text-xs opacity-50">~/.codex/${escapeHtml(g.file)}</div>
      ${g.error ? `<div class="mb-2 text-xs text-destructive">${escapeHtml(g.error)}</div>` : ""}
      <div class="flex flex-col gap-2">${params}</div>
      ${addBtn}
    </div>`;
  }).join("");
}

async function guardToggleBool(id: string): Promise<void> {
  const st = state.guardState.params[id];
  if (st?.locked) return;
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    await invoke("guard_set_value", { id, value: p.value !== true });
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Change failed: {{error}}", { error: String(e) }), "error");
  }
}

async function guardSetValue(id: string, input: HTMLInputElement | HTMLTextAreaElement): Promise<void> {
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    const value = p.valueType === "int" ? parseInt(input.value, 10) : input.value;
    if (p.valueType === "int" && Number.isNaN(value)) {
      toast(t("Please enter an integer"), "error");
      await refreshGuardView(true);
      return;
    }
    await invoke("guard_set_value", { id, value });
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Save failed: {{error}}", { error: String(e) }), "error");
    await refreshGuardView(true);
  }
}

async function guardApply(id: string): Promise<void> {
  try {
    await invoke("guard_apply", { id });
    toast(t("Applied"), "success");
  } catch (e) {
    toast(t("Apply failed: {{error}}", { error: String(e) }), "error");
  }
  await refreshGuardView(true);
}

async function guardToggleApplied(id: string): Promise<void> {
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    if (p.applied) {
      await guardDisable(id);
    } else {
      await guardApply(id);
    }
  } catch (e) {
    toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
    await refreshGuardView(true);
  }
}

async function guardDisable(id: string): Promise<void> {
  try {
    await invoke("guard_set_applied", { id, applied: false });
    toast(t("Disabled"), "info");
  } catch (e) {
    toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
  }
  await refreshGuardView(true);
}

async function guardSetLocked(id: string, locked: boolean): Promise<void> {
  try {
    await invoke("guard_set_locked", { id, locked });
    toast(locked ? t("Locked") : t("Unlocked"), locked ? "success" : "info");
  } catch (e) {
    toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
  }
  await refreshGuardView(true);
}

// ============ 文件管理 ============
let guardFiles: GuardFileView[] = [];
let guardAddParamFileId: string | null = null;

async function refreshGuardFiles(): Promise<void> {
  try {
    guardFiles = await invoke<GuardFileView[]>("guard_get_files");
    renderGuardFiles();
    // 首次（无检测记录）自动检测一次并落盘；之后直接读记录，不重复扫盘
    for (const f of guardFiles) {
      if (f.builtin && !f.detection) {
        await guardDetectFile(f.id, true);
      }
    }
  } catch (e) {
    console.error("加载文件列表失败", e);
  }
}

function renderGuardFileSelect(): void {
  const sel = document.getElementById("guard-add-file-select") as HTMLSelectElement | null;
  if (!sel) return;
  const prev = sel.value;
  sel.innerHTML = guardFiles.map((f) =>
    `<option value="${f.id}">${escapeHtml(f.name)} (${f.format})</option>`
  ).join("");
  if (guardAddParamFileId && guardFiles.some((f) => f.id === guardAddParamFileId)) {
    sel.value = guardAddParamFileId;
  } else if (prev && guardFiles.some((f) => f.id === prev)) {
    sel.value = prev;
  }
}

function renderGuardFiles(): void {
  // 设置页里的文件列表
  const container = document.getElementById("settings-guard-files");
  if (container) {
    if (guardFiles.length === 0) {
      container.textContent = t("No files yet");
    } else {
      const html = guardFiles.map((f) => {
        const delBtn = f.builtin
          ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" disabled>${t("Built-in")}</button>`
          : `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-destructive/50 bg-background text-destructive hover:bg-destructive/10 h-6 px-2 text-xs" data-action="guardRemoveFile" data-id="${f.id}">${t("Delete")}</button>`;
        const detectBtn = f.builtin
          ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="guardDetectFile" data-id="${f.id}">${t("Detect")}</button>`
          : "";
        const det = f.detection;
        const detText = det
          ? det.path === null
            ? t("Detection: file not found ({{at}})", { at: fmtTs(det.at) })
            : det.path === f.file
              ? t("Detection: path matches ({{at}})", { at: fmtTs(det.at) })
              : t("Detection: actually at {{path}} ({{at}})", { path: det.path, at: fmtTs(det.at) })
          : "";
        return `<div class="rounded-lg border border-border bg-card text-card-foreground p-3" data-file-id="${f.id}">
          <div class="flex items-center gap-2">
            <span class="text-sm font-medium">${escapeHtml(f.name)}</span>
            <span class="inline-flex items-center rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">${f.format}</span>
          </div>
          <div class="mt-1 font-mono text-xs opacity-60">~/.codex/${escapeHtml(f.file)}</div>
          ${detText ? `<div class="mt-1 text-xs opacity-60">${escapeHtml(detText)}</div>` : ""}
          <div class="mt-2 flex gap-2">
            ${detectBtn}
            <button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="guardEditFile" data-id="${f.id}">${t("Edit")}</button>
            ${delBtn}
          </div>
        </div>`;
      }).join("");
      container.innerHTML = html;
    }
  }
  // 添加参数表单里的文件下拉
  renderGuardFileSelect();
}

let editingFileId: string | null = null;

function toggleGuardFileForm(): void {
  const modal = document.getElementById("guard-file-modal");
  if (modal) modal.classList.toggle("hidden");
  editingFileId = null;
  const submit = document.getElementById("settings-guard-file-submit");
  if (submit) submit.textContent = t("Add");
  const title = document.getElementById("guard-file-modal-title");
  if (title) title.textContent = t("Add Guard File");
  // 编辑模式下格式不可改（后端 guard_update_file 不收 format），添加时恢复可选
  const formatSel = document.getElementById("settings-guard-file-format") as HTMLSelectElement | null;
  if (formatSel) formatSel.disabled = false;
}

function guardEditFile(id: string): void {
  const f = guardFiles.find((x) => x.id === id);
  if (!f) return;
  editingFileId = id;
  (document.getElementById("settings-guard-file-name") as HTMLInputElement).value = f.name;
  (document.getElementById("settings-guard-file-path") as HTMLInputElement).value = f.file;
  const formatSel = document.getElementById("settings-guard-file-format") as HTMLSelectElement;
  formatSel.value = f.format;
  formatSel.disabled = true;
  document.getElementById("settings-guard-file-submit")!.textContent = t("Save");
  document.getElementById("guard-file-modal-title")!.textContent = t("Edit Guard File");
  document.getElementById("guard-file-modal")!.classList.remove("hidden");
}

async function guardPickFilePath(): Promise<void> {
  try {
    const selected = await openDialog({ multiple: false });
    if (typeof selected !== "string") return;
    const rel = await invoke<string>("guard_relativize_picked_path", { absPath: selected });
    (document.getElementById("settings-guard-file-path") as HTMLInputElement).value = rel;
    // 顺手带入文件名与格式
    const nameEl = document.getElementById("settings-guard-file-name") as HTMLInputElement;
    const fileName = rel.split("/").pop() ?? rel;
    if (!nameEl.value.trim()) nameEl.value = fileName;
    const ext = fileName.split(".").pop()?.toLowerCase();
    if (ext === "toml" || ext === "json" || ext === "md") {
      (document.getElementById("settings-guard-file-format") as HTMLSelectElement).value = ext;
    }
  } catch (e) {
    toast(`${e}`, "error");
  }
}

async function guardSaveFileForm(): Promise<void> {
  const name = (document.getElementById("settings-guard-file-name") as HTMLInputElement).value.trim();
  const file = (document.getElementById("settings-guard-file-path") as HTMLInputElement).value.trim();
  const format = (document.getElementById("settings-guard-file-format") as HTMLSelectElement).value;
  if (!name) { toast(t("Please enter a file name"), "error"); return; }
  if (!file) { toast(t("Please enter a file path"), "error"); return; }
  try {
    if (editingFileId) {
      await invoke("guard_update_file", { id: editingFileId, name, file });
      toast(t("Updated"), "success");
    } else {
      await invoke("guard_add_file", { name, file, format });
      toast(t("File added"), "success");
    }
    (document.getElementById("settings-guard-file-name") as HTMLInputElement).value = "";
    (document.getElementById("settings-guard-file-path") as HTMLInputElement).value = "";
    toggleGuardFileForm();
    await refreshGuardFiles();
    await refreshGuardView(true);
  } catch (e) {
    toast(t(editingFileId ? "Update failed: {{error}}" : "Add failed: {{error}}", { error: String(e) }), "error");
  }
}

async function guardDetectFile(id: string, auto = false): Promise<void> {
  const f = guardFiles.find((x) => x.id === id);
  if (!f) return;
  try {
    const updated = await invoke<GuardFileView>("guard_detect_file", { id });
    guardFiles = guardFiles.map((x) => (x.id === id ? updated : x));
    renderGuardFiles();
    const detected = updated.detection?.path ?? null;
    if (detected && detected !== updated.file) {
      const ok = await ask(
        t("\"{{name}}\" was detected at:\n~/.codex/{{detected}}\n\nIt differs from the configured ~/.codex/{{file}}. Update to the detected path?", {
          name: updated.name, detected, file: updated.file,
        }),
        { title: t("Update Guard Path"), kind: "warning" }
      );
      if (ok) {
        await invoke("guard_update_file", { id, name: updated.name, file: detected });
        toast(t("Updated to the detected path"), "success");
        await refreshGuardFiles();
        await refreshGuardView(true);
      }
    } else if (!auto) {
      toast(detected ? t("Detection complete: path matches") : t("File not found under ~/.codex"), detected ? "success" : "info");
    }
  } catch (e) {
    if (!auto) toast(t("Detection failed: {{error}}", { error: String(e) }), "error");
  }
}

async function guardRemoveFile(id: string): Promise<void> {
  const f = guardFiles.find((x) => x.id === id);
  if (!f) return;
  if (!(await ask(t("Delete file \"{{name}}\"?\n\nAll custom parameters under it will be unguarded, but values already written to ~/.codex/{{file}} will not be rolled back.", { name: f.name, file: f.file }), { title: t("Delete Guard File"), kind: "warning" }))) {
    return;
  }
  try {
    await invoke("guard_remove_file", { id });
    toast(t("Deleted"), "success");
    await refreshGuardFiles();
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Delete failed: {{error}}", { error: String(e) }), "error");
  }
}

// ============ 自定义参数管理 ============
function openGuardAddFormFor(fileId: string): void {
  guardAddParamFileId = fileId;
  const fileSelect = document.getElementById("guard-add-file-select") as HTMLSelectElement | null;
  if (fileSelect) {
    fileSelect.value = fileId;
  }
  openGuardAddModal();
}

function openGuardAddModal(): void {
  document.getElementById("guard-add-modal")!.classList.remove("hidden");
  onGuardAddModeChange();
  onGuardAddValueTypeChange();
}

function closeGuardAddModal(): void {
  document.getElementById("guard-add-modal")!.classList.add("hidden");
}

function onGuardAddModeChange(): void {
  const mode = (document.getElementById("guard-add-mode") as HTMLSelectElement).value;
  const pathRow = document.getElementById("guard-add-path-row")!;
  const valueTypeRow = document.getElementById("guard-add-value-type-row")!;
  const defaultRow = document.getElementById("guard-add-default-row")!;

  const isToml = mode === "toml_key" || mode === "toml_absent";
  // file_overwrite / markdown_block 固定为 text 类型，无需选值类型
  pathRow.classList.toggle("hidden", !isToml);
  valueTypeRow.classList.toggle("hidden", !isToml);
  defaultRow.classList.toggle("hidden",
    isToml && (document.getElementById("guard-add-value-type") as HTMLSelectElement).value === "none");
}

function onGuardAddValueTypeChange(): void {
  const vt = (document.getElementById("guard-add-value-type") as HTMLSelectElement).value;
  const defaultRow = document.getElementById("guard-add-default-row")!;
  defaultRow.classList.toggle("hidden", vt === "none");

  const defaultEl = document.getElementById("guard-add-default") as HTMLInputElement;
  const defaultRowParent = defaultEl.parentElement!;
  // 替换 input 为 textarea 或反之
  if (vt === "text" && defaultEl.tagName === "INPUT") {
    const ta = document.createElement("textarea");
    ta.className = "guard-form-textarea";
    ta.id = "guard-add-default";
    ta.value = defaultEl.value;
    defaultRowParent.replaceChild(ta, defaultEl);
  } else if (vt !== "text" && defaultEl.tagName === "TEXTAREA") {
    const inp = document.createElement("input");
    inp.type = "text";
    inp.className = "h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-xs outline-none transition-colors placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";
    inp.id = "guard-add-default";
    inp.value = defaultEl.value;
    defaultRowParent.replaceChild(inp, defaultEl);
  }
}

function parseDefaultValue(value: string, effectiveType: string): unknown {
  switch (effectiveType) {
    case "bool":
      return value === "true";
    case "int": {
      const n = parseInt(value, 10);
      if (Number.isNaN(n)) throw new Error(t("Default value must be an integer"));
      return n;
    }
    case "string":
    case "text":
      return value;
    case "none":
      return null;
    default:
      return value;
  }
}

async function guardAddCustom(): Promise<void> {
  const id = (document.getElementById("guard-add-id") as HTMLInputElement).value.trim();
  const label = (document.getElementById("guard-add-label") as HTMLInputElement).value.trim();
  const fileSelect = document.getElementById("guard-add-file-select") as HTMLSelectElement;
  const fileId = fileSelect?.value || guardAddParamFileId;
  const mode = (document.getElementById("guard-add-mode") as HTMLSelectElement).value;
  const path = (document.getElementById("guard-add-path") as HTMLInputElement).value.trim();
  const valueType = (document.getElementById("guard-add-value-type") as HTMLSelectElement).value;
  const desc = (document.getElementById("guard-add-desc") as HTMLInputElement).value.trim();
  const defaultEl = document.getElementById("guard-add-default") as HTMLInputElement | HTMLTextAreaElement;
  const defaultRaw = defaultEl.value;

  if (!id) { toast(t("Please enter an ID"), "error"); return; }
  if (!label) { toast(t("Please enter a name"), "error"); return; }
  if (!fileId) { toast(t("Please select a target file"), "error"); return; }
  if ((mode === "toml_key" || mode === "toml_absent") && !path) {
    toast(t("Please enter a TOML path"), "error"); return;
  }

  try {
    const effectiveType = (mode === "file_overwrite" || mode === "markdown_block") ? "text" : valueType;
    const defaultVal = parseDefaultValue(defaultRaw, effectiveType);
    const param = {
      id,
      label,
      description: desc,
      file: "",
      applyMode: mode,
      path,
      valueType: effectiveType,
      default: defaultVal,
      custom: true,
    };
    await invoke("guard_add_custom_param", { param, fileId });
    toast(t("Custom parameter added"), "success");
    // 清空表单并收起
    (document.getElementById("guard-add-id") as HTMLInputElement).value = "";
    (document.getElementById("guard-add-label") as HTMLInputElement).value = "";
    (document.getElementById("guard-add-path") as HTMLInputElement).value = "";
    (document.getElementById("guard-add-desc") as HTMLInputElement).value = "";
    defaultEl.value = "";
    guardAddParamFileId = null;
    closeGuardAddModal();
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Add failed: {{error}}", { error: String(e) }), "error");
  }
}

async function guardRemoveCustom(id: string): Promise<void> {
  if (!(await ask(t("Delete custom parameter {{id}}?\n\nGuarding stops after deletion. Values already written to ~/.codex/ will not be rolled back; restore manually from ~/.codex/dashi-backups/ if needed.", { id }), { title: t("Delete Custom Parameter"), kind: "warning" }))) {
    return;
  }
  try {
    await invoke("guard_remove_custom_param", { id });
    toast(t("Deleted"), "success");
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Delete failed: {{error}}", { error: String(e) }), "error");
  }
}

async function guardOpenSchemaFile(): Promise<void> {
  try {
    const path = await invoke<string>("guard_get_schema_file_path");
    // shell 插件的 open 可以打开文件所在目录/文件
    await openUrl(path);
  } catch (e) {
    // 回退：复制路径到剪贴板
    try {
      const path = await invoke<string>("guard_get_schema_file_path");
      await navigator.clipboard.writeText(path);
      toast(t("Path copied to clipboard: {{path}}", { path }), "info");
    } catch {
      toast(t("Open failed: {{error}}", { error: String(e) }), "error");
    }
  }
}

// ============ 更新检查 ============
async function checkUpdaterHealth(): Promise<void> {
  const el = document.getElementById("updater-health")!;
  const helpRow = document.getElementById("updater-help-row")!;
  try {
    const health = await invoke<UpdaterConfigHealth>("get_updater_config_health");
    if (health.configured) {
      el.textContent = t("Ready");
      el.className = "health-status ok";
      helpRow.classList.add("hidden");
    } else {
      el.textContent = health.message;
      el.className = "health-status err";
      helpRow.classList.remove("hidden");
    }
  } catch (e) {
    el.textContent = t("Check failed: {{error}}", { error: String(e) });
    el.className = "health-status err";
    helpRow.classList.remove("hidden");
  }
}

async function openUpdaterHelp(target: "docs" | "template"): Promise<void> {
  try {
    const paths = await invoke<UpdaterHelpPaths>("get_updater_help_paths");
    await openUrl(target === "docs" ? paths.docsPath : paths.templatePath);
  } catch (e) {
    toast(t("Failed to open help: {{error}}", { error: String(e) }), "error");
  }
}

let pendingUpdateInfo: UpdateInfo | null = null;
let updateBusy = false;

function renderUpdateInfo(info: UpdateInfo): void {
  pendingUpdateInfo = info.hasUpdate ? info : null;
  const row = document.getElementById("update-available-row")!;
  const btn = document.getElementById("btn-check-update")! as HTMLButtonElement;
  if (info.hasUpdate && info.availableVersion) {
    row.classList.remove("hidden");
    document.getElementById("update-version")!.textContent = `v${info.availableVersion}`;
    const notes = document.getElementById("update-notes")!;
    notes.textContent = info.releaseNotes?.trim() || "";
    notes.classList.toggle("hidden", !notes.textContent);
    btn.textContent = t("Update Now");
  } else {
    row.classList.add("hidden");
    btn.textContent = t("Check for Updates");
  }
}

function renderDownloadProgress(p: DownloadProgress): void {
  const row = document.getElementById("update-progress-row")!;
  row.classList.remove("hidden");
  const bar = document.getElementById("update-progress-bar")!;
  const text = document.getElementById("update-progress-text")!;
  if (p.stage === "restarting") {
    bar.style.width = "100%";
    text.textContent = t("Installation complete, restarting…");
  } else if (p.stage === "installing") {
    bar.style.width = "100%";
    text.textContent = t("Installing…");
  } else if (p.stage === "retrying") {
    text.textContent = t("Download failed, retrying ({{attempt}}/{{max}})…", { attempt: p.attempt, max: p.maxAttempts });
  } else {
    if (p.percent !== null) {
      bar.style.width = `${p.percent}%`;
      text.textContent = t("Downloading v{{version}}: {{percent}}%", { version: p.version, percent: Math.floor(p.percent) });
    } else {
      const mb = (p.downloadedBytes / 1024 / 1024).toFixed(1);
      text.textContent = t("Downloading v{{version}}: {{mb}} MB", { version: p.version, mb });
    }
  }
}

async function checkUpdate(silent = false): Promise<void> {
  if (updateBusy) return;
  updateBusy = true;
  const btn = document.getElementById("btn-check-update")! as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = t("Checking...");
  try {
    const info = await invoke<UpdateInfo>("check_update");
    renderUpdateInfo(info);
    if (info.hasUpdate) {
      toast(t("New version available: v{{version}}", { version: String(info.availableVersion) }), "info");
    } else if (info.message) {
      if (!silent) toast(info.message, "error");
    } else if (!silent) {
      toast(t("Already up to date"), "info");
    }
  } catch (e) {
    if (!silent) toast(t("Failed to check for updates: {{error}}", { error: String(e) }), "error");
  } finally {
    updateBusy = false;
    btn.disabled = false;
    if (!pendingUpdateInfo) btn.textContent = t("Check for Updates");
  }
}

async function onUpdateButton(): Promise<void> {
  if (!pendingUpdateInfo) {
    await checkUpdate();
    return;
  }
  if (updateBusy) return;
  updateBusy = true;
  const btn = document.getElementById("btn-check-update")! as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = t("Updating...");
  try {
    const msg = await invoke<string>("install_update", {
      expectedVersion: pendingUpdateInfo.availableVersion,
    });
    toast(msg, "success");
    pendingUpdateInfo = null;
    document.getElementById("update-available-row")!.classList.add("hidden");
    btn.textContent = t("Check for Updates");
  } catch (e) {
    toast(t("Update failed: {{error}}", { error: String(e) }), "error");
    btn.textContent = t("Update Now");
  } finally {
    updateBusy = false;
    btn.disabled = false;
    document.getElementById("update-progress-row")!.classList.add("hidden");
    document.getElementById("update-progress-bar")!.style.width = "0%";
  }
}

// ============ GitHub 链接 ============
async function openGithub(): Promise<void> {
  try {
    await openUrl("https://github.com");
  } catch (e) {
    toast(t("Failed to open link: {{error}}", { error: String(e) }), "error");
  }
}

// ============ 状态更新 ============
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

async function refreshStatus(): Promise<void> {
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

function updateStatusUI(info: ProcessInfo): void {
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

// ============ 事件监听 ============
async function setupEventListener(): Promise<void> {
  await listen<{ name: string; status: string; message: string }>("status-update", (event) => {
    const payload = event.payload;
    const info: ProcessInfo = {
      name: payload.name,
      status: payload.status as ProcessInfo["status"],
      pid: null,
      message: payload.message,
    };
    updateStatusUI(info);
  });
  await listen<DownloadProgress>("updater-download-progress", (event) => {
    renderDownloadProgress(event.payload);
  });
}

// ============ 初始化 ============
let statusPolling: ReturnType<typeof setInterval> | null = null;

async function init(): Promise<void> {
  // 主题必须在首个 await 前同步应用，避免首屏按默认色绘制后再闪切。
  applyTheme(getStoredTheme());

  // 初始化 i18n（先于一切 UI 渲染）：语言在 Rust 启动时已解析好
  try {
    const resolved = await invoke<string>("get_resolved_language");
    initI18n(resolved === "zh-CN" ? "zh-CN" : "en");
  } catch {
    initI18n("en");
  }
  document.documentElement.lang = currentLanguage();
  applyDomTranslations();

  // 事件接线（在所有 UI 交互之前）
  wireEvents();

  // 主题族网格 + 应用主题
  renderThemeFamilyGrid();
  applyTheme(getStoredTheme());

  // 监听系统主题变化
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (getStoredTheme() === "system") {
      applyTheme("system");
    }
  });

  try {
    // 加载配置并填充 UI
    const cfg = await invoke<LauncherConfig>("load_config");
    fillConfigUI(cfg);

    // 自启状态从 OS 注册项实时读（不存配置）
    try {
      const autostart = await invoke<boolean>("autostart_is_enabled");
      (document.getElementById("toggle-autostart") as HTMLInputElement).checked = autostart;
    } catch { /* 读不到就当关 */ }

    // 进程事故通知需要系统授权（macOS），启动时静默请求一次
    void (async () => {
      try {
        if (!(await isPermissionGranted())) await requestPermission();
      } catch { /* 拒绝则通知静默失败，不打扰 */ }
    })();

    // codex 路径为空或已失效时，自动探测真实安装位置并回填
    const codexInput = document.getElementById("cfg-codex") as HTMLInputElement;
    const currentValid = codexInput.value
      && await invoke<boolean>("check_codex_app", { appPath: codexInput.value });
    if (!currentValid) {
      const found = await invoke<string | null>("detect_codex_app");
      if (found) {
        codexInput.value = found;
        await invoke("update_settings", { config: readConfigFromUI() });
      }
    }

    await validatePaths();

    // 获取应用版本
    try {
      const version = await getVersion();
      document.getElementById("about-version")!.textContent = version;
    } catch {
      document.getElementById("about-version")!.textContent = "unknown";
    }

    // 检查更新源健康状态
    await checkUpdaterHealth();

    // 静默检查更新，有新版本时提示
    void checkUpdate(true);

    // 设置事件监听
    await setupEventListener();

    // 刷新状态
    await refreshStatus();

    // 启动状态轮询（每 3 秒）
    if (statusPolling !== null) {
      clearInterval(statusPolling);
    }
    statusPolling = setInterval(() => {
      void refreshStatus();
      void refreshGuardView();
    }, 3000);
  } catch (e) {
    toast(t("Initialization failed: {{error}}", { error: String(e) }), "error");
  }
}

// ============ 事件接线（inline handler 已全部移除，CSP script-src 不再需要 unsafe-inline） ============
// 视图刷新触发集中在这里（ADR 0009：跨域通信由 shell 的事件接线完成；nav.ts 只做纯 DOM 切换）
function on(id: string, event: string, handler: (ev: Event) => void): void {
  document.getElementById(id)?.addEventListener(event, handler);
}

// 动态渲染内容（看守参数/文件卡）的事件委托：data-action / data-change-action + data-id
function delegate(containerId: string, event: "click" | "change", handlers: Record<string, (el: HTMLElement) => void>): void {
  const attr = event === "change" ? "changeAction" : "action";
  document.getElementById(containerId)?.addEventListener(event, (ev) => {
    const el = (ev.target as HTMLElement).closest<HTMLElement>(`[data-${attr === "changeAction" ? "change-action" : "action"}]`);
    if (!el) return;
    const fn = handlers[el.dataset[attr]!];
    if (fn) fn(el);
  });
}

function wireEvents(): void {
  // 顶部导航（进入视图后的数据刷新在调用点触发）
  on("btn-home", "click", showHome);
  on("btn-skill", "click", () => {
    showSkill();
    void refreshSkillStatus();
  });
  on("btn-guard", "click", () => {
    showGuard();
    void refreshGuardView(true);
    void refreshGuardFiles();
  });
  on("btn-settings", "click", toggleSettings);

  // 主页
  on("btn-start-all", "click", () => void startAll());
  on("btn-stop-all", "click", () => void stopAll());
  on("btn-start-tb", "click", () => void startTaskboard());
  on("btn-stop-tb", "click", () => void stopTaskboard());
  on("btn-open-tb", "click", () => void openTaskboard());
  on("btn-start-inj", "click", () => void startInjector());
  on("btn-stop-inj", "click", () => void stopInjector());

  // 设置侧栏（guard/integration 分区的数据刷新在调用点触发）
  for (const s of ["general", "appearance", "network", "mode", "guard", "integration", "about"]) {
    on(`nav-${s}`, "click", () => {
      switchSection(s);
      if (s === "guard") void refreshGuardFiles();
      if (s === "integration") void refreshFastctxStatus();
    });
  }

  // 语言/主题
  for (const l of ["system", "en", "zh-CN"]) {
    on(`lang-card-${l}`, "click", () => void setLanguage(l));
  }
  for (const m of ["system", "light", "dark"] as ThemeMode[]) {
    on(`theme-card-${m}`, "click", () => setTheme(m));
  }
  document.getElementById("theme-family-grid")?.addEventListener("click", (ev) => {
    const card = (ev.target as HTMLElement).closest<HTMLElement>("[data-family]");
    if (card?.dataset.family) setThemeFamily(card.dataset.family);
  });

  // 通用设置
  on("btn-browse-path", "click", () => void browsePath());
  on("btn-browse-node", "click", () => void browseNode());
  on("btn-browse-codex", "click", () => void browseCodex());
  on("btn-use-bundled", "click", () => void useBundledTaskboard());
  on("btn-open-logs", "click", () => void openLogDir());
  for (const id of ["cfg-path", "cfg-node", "cfg-codex", "cfg-host", "cfg-port", "cfg-cdp"]) {
    on(id, "input", onConfigChange);
  }
  on("toggle-tray", "change", onConfigChange);
  on("toggle-autostart", "change", () => void toggleAutostart());
  on("toggle-mode", "change", () => { updateModeLabel(); onConfigChange(); });
  on("toggle-auto-open", "change", () => { updateAutoOpenLabel(); onConfigChange(); });

  // 看守/集成
  on("settings-guard-toggle", "change", () => void toggleGuard());
  on("guard-file-form-toggle", "click", toggleGuardFileForm);
  on("toggle-fastctx", "change", () => void toggleFastctx());
  on("btn-fastctx-console", "click", () => void openFastctxConsole());

  // 关于
  on("link-updater-docs", "click", () => void openUpdaterHelp("docs"));
  on("link-updater-template", "click", () => void openUpdaterHelp("template"));
  on("btn-check-update", "click", () => void onUpdateButton());
  on("link-github", "click", () => void openGithub());

  // 保存
  on("btn-save-config", "click", () => void saveConfig());

  // Skill
  on("btn-install-skill", "click", () => void installSkill());

  // 看守视图（静态部分）
  on("guard-add-toggle", "click", openGuardAddModal);
  on("guard-add-cancel", "click", closeGuardAddModal);
  on("guard-add-submit", "click", () => void guardAddCustom());
  on("guard-add-mode", "change", onGuardAddModeChange);
  on("guard-add-value-type", "change", onGuardAddValueTypeChange);
  on("btn-guard-open-schema", "click", () => void guardOpenSchemaFile());
  document.getElementById("guard-add-modal")?.addEventListener("click", (ev) => {
    if (ev.target === ev.currentTarget) closeGuardAddModal();
  });

  // 文件弹窗
  on("guard-file-cancel", "click", toggleGuardFileForm);
  on("settings-guard-file-submit", "click", () => void guardSaveFileForm());
  on("btn-guard-pick-path", "click", () => void guardPickFilePath());
  document.getElementById("guard-file-modal")?.addEventListener("click", (ev) => {
    if (ev.target === ev.currentTarget) toggleGuardFileForm();
  });

  // 动态内容委托
  delegate("guard-view", "click", {
    guardApply: (el) => void guardApply(el.dataset.id!),
    guardDisable: (el) => void guardDisable(el.dataset.id!),
    guardSetLocked: (el) => void guardSetLocked(el.dataset.id!, el.dataset.locked === "true"),
    guardRemoveCustom: (el) => void guardRemoveCustom(el.dataset.id!),
    openGuardAddFormFor: (el) => openGuardAddFormFor(el.dataset.id!),
  });
  delegate("guard-view", "change", {
    guardToggleBool: (el) => void guardToggleBool(el.dataset.id!),
    guardToggleApplied: (el) => void guardToggleApplied(el.dataset.id!),
    guardSetValue: (el) => void guardSetValue(el.dataset.id!, el as HTMLInputElement),
  });
  delegate("settings-guard-files", "click", {
    guardRemoveFile: (el) => void guardRemoveFile(el.dataset.id!),
    guardDetectFile: (el) => void guardDetectFile(el.dataset.id!),
    guardEditFile: (el) => guardEditFile(el.dataset.id!),
  });
}

init();
