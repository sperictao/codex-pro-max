import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { open as openDialog, ask } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";

// ============ 类型定义 ============
interface LauncherConfig {
  taskboard_path: string;
  node_path: string;
  codex_app_path: string;
  taskboard_port: number;
  taskboard_host: string;
  cdp_port: number;
  auto_open: boolean;
  separate_window_mode: boolean;
  minimize_to_tray_on_close: boolean;
  codex_guard: CodexGuardState;
}

interface GuardParamState {
  value: unknown | null;
  applied: boolean;
  locked: boolean;
  last_checked?: number | null;
  last_restored?: number | null;
}

interface CodexGuardState {
  enabled: boolean;
  params: Record<string, GuardParamState>;
}

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

type ThemeMode = "light" | "dark" | "system";

// ============ 全局状态 ============
let statusPolling: ReturnType<typeof setInterval> | null = null;
let guardState: CodexGuardState = { enabled: false, params: {} };
let lastGuardJson = "";
let fastctxState: FastctxStatus = { installed: false, version: null, integrated: false };
let fastctxBusy = false;

// ============ Toast 通知 ============
function toast(message: string, type: "success" | "error" | "info" = "info"): void {
  const container = document.getElementById("toast-container")!;
  const el = document.createElement("div");
  el.className = `toast ${type}`;
  el.textContent = message;
  container.appendChild(el);
  setTimeout(() => {
    el.style.opacity = "0";
    el.style.transition = "opacity 0.3s";
    setTimeout(() => el.remove(), 300);
  }, 3000);
}

// ============ 主题管理 ============
function getStoredTheme(): ThemeMode {
  const stored = localStorage.getItem("theme");
  if (stored === "light" || stored === "dark" || stored === "system") {
    return stored;
  }
  return "system";
}

function applyTheme(mode: ThemeMode): void {
  const html = document.documentElement;
  const isDark =
    mode === "dark" ||
    (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);

  if (isDark) {
    html.classList.add("dark");
  } else {
    html.classList.remove("dark");
  }

  for (const m of ["light", "dark", "system"] as ThemeMode[]) {
    document.getElementById(`theme-card-${m}`)?.classList.toggle("selected", m === mode);
  }
}

function setTheme(mode: ThemeMode): void {
  localStorage.setItem("theme", mode);
  applyTheme(mode);
}

// ============ 配置管理 ============
function fillConfigUI(cfg: LauncherConfig): void {
  (document.getElementById("cfg-path") as HTMLInputElement).value = cfg.taskboard_path;
  (document.getElementById("cfg-node") as HTMLInputElement).value = cfg.node_path;
  (document.getElementById("cfg-codex") as HTMLInputElement).value = cfg.codex_app_path;
  (document.getElementById("cfg-host") as HTMLInputElement).value = cfg.taskboard_host;
  (document.getElementById("cfg-port") as HTMLInputElement).value = String(cfg.taskboard_port);
  (document.getElementById("cfg-cdp") as HTMLInputElement).value = String(cfg.cdp_port);

  const modeToggle = document.getElementById("toggle-mode")!;
  if (cfg.separate_window_mode) {
    modeToggle.classList.add("active");
  } else {
    modeToggle.classList.remove("active");
  }
  updateModeLabel();

  const autoOpenToggle = document.getElementById("toggle-auto-open")!;
  if (cfg.auto_open) {
    autoOpenToggle.classList.add("active");
  } else {
    autoOpenToggle.classList.remove("active");
  }
  updateAutoOpenLabel();

  document.getElementById("toggle-tray")!.classList.toggle("active", cfg.minimize_to_tray_on_close);

  guardState = cfg.codex_guard ?? { enabled: false, params: {} };
  renderGuardToggle();
}

function renderGuardToggle(): void {
  const el = document.getElementById("settings-guard-toggle");
  if (el) el.classList.toggle("active", guardState.enabled);
  // 总开关关闭时隐藏顶部「看守」Tab
  const btn = document.getElementById("btn-guard");
  if (btn) btn.classList.toggle("hidden", !guardState.enabled);
  // 如果关了总开关且当前在看守页，跳回主页
  if (!guardState.enabled) {
    const view = document.getElementById("guard-view");
    if (view && !view.classList.contains("hidden")) {
      showHome();
    }
  }
}

function readConfigFromUI(): LauncherConfig {
  return {
    taskboard_path: (document.getElementById("cfg-path") as HTMLInputElement).value,
    node_path: (document.getElementById("cfg-node") as HTMLInputElement).value,
    codex_app_path: (document.getElementById("cfg-codex") as HTMLInputElement).value,
    taskboard_host: (document.getElementById("cfg-host") as HTMLInputElement).value || "127.0.0.1",
    taskboard_port: parseInt((document.getElementById("cfg-port") as HTMLInputElement).value) || 47823,
    cdp_port: parseInt((document.getElementById("cfg-cdp") as HTMLInputElement).value) || 9231,
    auto_open: document.getElementById("toggle-auto-open")!.classList.contains("active"),
    separate_window_mode: document.getElementById("toggle-mode")!.classList.contains("active"),
    minimize_to_tray_on_close: document.getElementById("toggle-tray")!.classList.contains("active"),
    codex_guard: guardState,
  };
}

function toggleTrayMinimize(): void {
  document.getElementById("toggle-tray")!.classList.toggle("active");
  onConfigChange();
}

function onConfigChange(): void {
  validatePaths();
}

async function saveConfig(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    // 使用 update_settings 而非 save_config：只更新设置类字段，
    // 保留 codex_guard 等看守状态不变，避免设置页保存回滚 apply/lock 状态
    await invoke("update_settings", { config: cfg });
    // 保存后同步后端最新的完整配置到前端 guardState，保持一致
    const latest = await invoke<LauncherConfig>("load_config");
    guardState = latest.codex_guard;
    toast("配置已保存", "success");
  } catch (e) {
    toast(`保存失败: ${e}`, "error");
  }
}

// ============ 路径验证 ============
async function validatePaths(): Promise<void> {
  const cfg = readConfigFromUI();

  // 验证 taskboard 路径
  const pathEl = document.getElementById("validate-path")!;
  if (cfg.taskboard_path) {
    try {
      const valid = await invoke<boolean>("validate_taskboard_path", { path: cfg.taskboard_path });
      pathEl.textContent = valid ? "有效" : "无效";
      pathEl.className = `config-validate ${valid ? "ok" : "err"}`;
    } catch {
      pathEl.textContent = "检查失败";
      pathEl.className = "config-validate err";
    }
  } else {
    pathEl.textContent = "";
    pathEl.className = "config-validate";
  }

  // 验证 node（空路径也检查，因为会用系统 node）
  const nodeEl = document.getElementById("validate-node")!;
  try {
    const version = await invoke<string>("check_node_version", { nodePath: cfg.node_path });
    nodeEl.textContent = version;
    nodeEl.className = "config-validate ok";
  } catch {
    nodeEl.textContent = "不可用";
    nodeEl.className = "config-validate err";
  }

  // 验证 codex app
  const codexEl = document.getElementById("validate-codex")!;
  if (cfg.codex_app_path) {
    try {
      const exists = await invoke<boolean>("check_codex_app", { appPath: cfg.codex_app_path });
      codexEl.textContent = exists ? "存在" : "不存在";
      codexEl.className = `config-validate ${exists ? "ok" : "err"}`;
    } catch {
      codexEl.textContent = "检查失败";
      codexEl.className = "config-validate err";
    }
  } else {
    codexEl.textContent = "";
    codexEl.className = "config-validate";
  }
}

// ============ 使用内置 Taskboard ============
async function useBundledTaskboard(): Promise<void> {
  try {
    const path = await invoke<string | null>("get_bundled_taskboard_path");
    if (path) {
      (document.getElementById("cfg-path") as HTMLInputElement).value = path;
      onConfigChange();
      toast("已使用内置 Taskboard 路径", "success");
    } else {
      toast("未找到内置 Taskboard", "error");
    }
  } catch (e) {
    toast(`获取内置路径失败: ${e}`, "error");
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

// ============ 模式切换 ============
function toggleMode(): void {
  const toggle = document.getElementById("toggle-mode")!;
  toggle.classList.toggle("active");
  updateModeLabel();
  onConfigChange();
}

function updateModeLabel(): void {
  const toggle = document.getElementById("toggle-mode")!;
  const label = document.getElementById("toggle-mode-label")!;
  if (toggle.classList.contains("active")) {
    label.textContent = "独立窗口模式（不重启 Codex）";
  } else {
    label.textContent = "完整启动模式（重启 Codex）";
  }
}

function toggleAutoOpen(): void {
  const toggle = document.getElementById("toggle-auto-open")!;
  toggle.classList.toggle("active");
  updateAutoOpenLabel();
  onConfigChange();
}

function updateAutoOpenLabel(): void {
  const toggle = document.getElementById("toggle-auto-open")!;
  const label = document.getElementById("toggle-auto-open-label")!;
  if (toggle.classList.contains("active")) {
    label.textContent = "启动时自动打开浏览器";
  } else {
    label.textContent = "不自动打开浏览器";
  }
}

// ============ 视图切换 ============
function toggleSettings(): void {
  const mainView = document.getElementById("main-view")!;
  const settingsView = document.getElementById("settings-view")!;
  const btn = document.getElementById("btn-settings")!;
  const homeBtn = document.getElementById("btn-home")!;

  const isHidden = settingsView.classList.contains("hidden");

  if (isHidden) {
    mainView.classList.add("hidden");
    document.getElementById("skill-view")!.classList.add("hidden");
    document.getElementById("guard-view")!.classList.add("hidden");
    document.getElementById("btn-skill")!.classList.remove("active");
    document.getElementById("btn-guard")!.classList.remove("active");
    settingsView.classList.remove("hidden");
    btn.classList.add("active");
    homeBtn.classList.remove("active");
  } else {
    showHome();
  }
}

function showHome(): void {
  document.getElementById("main-view")!.classList.remove("hidden");
  document.getElementById("settings-view")!.classList.add("hidden");
  document.getElementById("skill-view")!.classList.add("hidden");
  document.getElementById("guard-view")!.classList.add("hidden");
  document.getElementById("btn-settings")!.classList.remove("active");
  document.getElementById("btn-skill")!.classList.remove("active");
  document.getElementById("btn-guard")!.classList.remove("active");
  document.getElementById("btn-home")!.classList.add("active");
}

function showSkill(): void {
  document.getElementById("main-view")!.classList.add("hidden");
  document.getElementById("settings-view")!.classList.add("hidden");
  document.getElementById("guard-view")!.classList.add("hidden");
  document.getElementById("skill-view")!.classList.remove("hidden");
  document.getElementById("btn-settings")!.classList.remove("active");
  document.getElementById("btn-home")!.classList.remove("active");
  document.getElementById("btn-guard")!.classList.remove("active");
  document.getElementById("btn-skill")!.classList.add("active");
  void refreshSkillStatus();
}

function showGuard(): void {
  document.getElementById("main-view")!.classList.add("hidden");
  document.getElementById("settings-view")!.classList.add("hidden");
  document.getElementById("skill-view")!.classList.add("hidden");
  document.getElementById("guard-view")!.classList.remove("hidden");
  document.getElementById("btn-settings")!.classList.remove("active");
  document.getElementById("btn-home")!.classList.remove("active");
  document.getElementById("btn-skill")!.classList.remove("active");
  document.getElementById("btn-guard")!.classList.add("active");
  void refreshGuardView(true);
  void refreshGuardFiles();
}

function switchSection(section: string): void {
  document.querySelectorAll(".settings-section").forEach((el) => {
    el.classList.add("hidden");
  });
  document.getElementById(`section-${section}`)!.classList.remove("hidden");

  document.querySelectorAll(".nav-item").forEach((el) => {
    el.classList.remove("active");
  });
  document.getElementById(`nav-${section}`)!.classList.add("active");

  const footer = document.getElementById("settings-footer")!;
  if (section === "about" || section === "appearance" || section === "guard" || section === "integration") {
    footer.classList.add("hidden");
  } else {
    footer.classList.remove("hidden");
  }

  if (section === "guard") {
    void refreshGuardFiles();
  }

  if (section === "integration") {
    void refreshFastctxStatus();
  }
}

// ============ 启动/停止 ============
async function startAll(): Promise<void> {
  const cfg = readConfigFromUI();
  if (!cfg.taskboard_path) {
    toast("请先在设置中配置 Taskboard 路径", "error");
    const settingsView = document.getElementById("settings-view")!;
    if (settingsView.classList.contains("hidden")) {
      toggleSettings();
    }
    switchSection("general");
    return;
  }

  const btn = document.getElementById("btn-start-all")! as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = "启动中...";

  try {
    await invoke("update_settings", { config: cfg });
    await invoke("start_all", { config: cfg });
    toast("所有服务已启动", "success");
    await refreshStatus();
  } catch (e) {
    toast(`启动失败: ${e}`, "error");
  } finally {
    btn.disabled = false;
    btn.textContent = "一键启动";
  }
}

async function stopAll(): Promise<void> {
  const btn = document.getElementById("btn-stop-all")! as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = "停止中...";

  try {
    await invoke("stop_all");
    toast("所有服务已停止", "info");
    await refreshStatus();
  } catch (e) {
    toast(`停止失败: ${e}`, "error");
  } finally {
    btn.disabled = false;
    btn.textContent = "全部停止";
  }
}

async function startTaskboard(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    await invoke("start_taskboard", { config: cfg });
    toast("Taskboard 服务已启动", "success");
    await refreshStatus();
  } catch (e) {
    toast(`启动失败: ${e}`, "error");
  }
}

async function stopTaskboard(): Promise<void> {
  try {
    await invoke("stop_taskboard");
    toast("Taskboard 服务已停止", "info");
    await refreshStatus();
  } catch (e) {
    toast(`停止失败: ${e}`, "error");
  }
}

async function startInjector(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    await invoke("start_injector", { config: cfg });
    toast("Codex 注入器已启动", "success");
    await refreshStatus();
  } catch (e) {
    toast(`启动失败: ${e}`, "error");
  }
}

async function stopInjector(): Promise<void> {
  try {
    await invoke("stop_injector");
    toast("Codex 注入器已停止", "info");
    await refreshStatus();
  } catch (e) {
    toast(`停止失败: ${e}`, "error");
  }
}

async function openTaskboard(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    await invoke("open_taskboard", { config: cfg });
  } catch (e) {
    toast(`打开失败: ${e}`, "error");
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
      status.state === "installed" ? "已安装" : status.state === "mismatch" ? "安装异常" : "未安装";
    detail.textContent = status.detail;
  } catch (e) {
    badge.className = "status-badge failed";
    text.textContent = "检测失败";
    detail.textContent = String(e);
  }
}

async function installSkill(): Promise<void> {
  const cfg = readConfigFromUI();
  if (!cfg.taskboard_path) {
    toast("请先配置 Taskboard 路径", "error");
    return;
  }
  try {
    const result = await invoke<string>("install_skill", { taskboardPath: cfg.taskboard_path });
    document.getElementById("skill-result")!.textContent = result;
    toast("Skill 安装成功", "success");
  } catch (e) {
    document.getElementById("skill-result")!.textContent = `失败: ${e}`;
    toast(`安装失败: ${e}`, "error");
  }
  await refreshSkillStatus();
}

// ============ Codex 配置看守 ============
async function toggleGuard(): Promise<void> {
  const enabled = !guardState.enabled;
  try {
    await invoke("guard_set_enabled", { enabled });
    guardState.enabled = enabled;
    renderGuardToggle();
    toast(enabled ? "配置看守已开启" : "配置看守已关闭", enabled ? "success" : "info");
  } catch (e) {
    toast(`切换失败: ${e}`, "error");
  }
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

// ============ FastCtx 集成 ============
// 接入/摘除委托 fastctx CLI（ADR 0003）；状态以 config.toml 为准实时检测，不持久化开关
function renderFastctx(): void {
  document.getElementById("toggle-fastctx")!.classList.toggle("active", fastctxState.integrated);
  const status = document.getElementById("fastctx-status")!;
  const hint = document.getElementById("fastctx-install-hint")!;
  if (fastctxBusy) {
    status.textContent = "处理中…";
  } else if (!fastctxState.installed) {
    status.textContent = "未安装";
  } else if (fastctxState.integrated) {
    status.textContent = `已接入${fastctxState.version ? ` · ${fastctxState.version}` : ""}`;
  } else {
    status.textContent = `已安装${fastctxState.version ? `（${fastctxState.version}）` : ""}，未接入`;
  }
  hint.classList.toggle("hidden", fastctxState.installed);
}

async function refreshFastctxStatus(): Promise<void> {
  try {
    fastctxState = await invoke<FastctxStatus>("fastctx_detect");
  } catch (e) {
    toast(`fastctx 检测失败: ${e}`, "error");
  }
  renderFastctx();
}

async function toggleFastctx(): Promise<void> {
  if (fastctxBusy) return;
  if (!fastctxState.installed) {
    toast("未检测到 fastctx，请先运行 npm install --global fastctx", "error");
    return;
  }
  if (fastctxState.integrated) {
    const ok = await ask(
      "摘除将停止 fastctx 进程并删除 ~/.fastctx 受管数据（npm 包保留，可随时重新接入）。已写入的 Codex 配置会被移除。\n\n确定摘除？",
      { title: "摘除 fastctx", kind: "warning" },
    );
    if (!ok) return;
  }
  fastctxBusy = true;
  renderFastctx();
  try {
    if (fastctxState.integrated) {
      await invoke("fastctx_unapply");
      toast("fastctx 已摘除，重启 Codex 会话后完全生效", "info");
    } else {
      const res = await invoke<FastctxApplyResult>("fastctx_apply");
      toast("fastctx 已接入，请重启 Codex 会话使其生效", "success");
      if (!res.selfCheckPassed) {
        const line = res.selfCheckOutput.split("\n").find((l) => l.includes("[FAIL]")) ?? res.selfCheckOutput.split("\n")[0] ?? "";
        toast(`fastctx 自检未通过：${line}（可打开控制台排查）`, "error");
      }
    }
  } catch (e) {
    toast(`fastctx 操作失败: ${e}`, "error");
  } finally {
    fastctxBusy = false;
    await refreshFastctxStatus();
  }
}

async function openFastctxConsole(): Promise<void> {
  if (!fastctxState.installed) {
    toast("未检测到 fastctx，请先运行 npm install --global fastctx", "error");
    return;
  }
  try {
    await invoke("fastctx_open_console");
  } catch (e) {
    toast(`打开控制台失败: ${e}`, "error");
  }
}

function fmtTs(ts: number | null): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString("zh-CN", { hour12: false });
}

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

function renderGuardView(view: GuardView): void {
  const container = document.getElementById("guard-groups")!;
  const statusMap: Record<string, { text: string; cls: string }> = {
    match: { text: "一致", cls: "running" },
    drift: { text: "不一致", cls: "failed" },
    missing: { text: "缺失", cls: "starting" },
    error: { text: "错误", cls: "failed" },
  };
  container.innerHTML = view.groups.map((g) => {
    const params = g.params.map((p) => {
      const s = statusMap[p.status] ?? statusMap.error;
      let editor = "";
      const dis = p.locked ? "disabled" : "";
      if (p.valueType === "bool") {
        editor = `<div class="guard-bool-row">
          <div class="toggle-switch ${p.value === true ? "active" : ""} ${p.locked ? "disabled" : ""}"
               onclick="guardToggleBool('${p.id}')"></div>
          <span class="guard-bool-label">${p.value === true ? "true" : "false"}（推荐 ${p.default}）</span>
        </div>`;
      } else if (p.valueType === "int" || p.valueType === "string") {
        const t = p.valueType === "int" ? "number" : "text";
        editor = `<input type="${t}" class="config-input mono guard-value-input" ${dis}
               value="${escapeHtml(String(p.value ?? ""))}" data-guard-id="${p.id}"
               onchange="guardSetValue('${p.id}', this)" />`;
      } else if (p.valueType === "text") {
        editor = `<textarea class="guard-textarea" ${dis} data-guard-id="${p.id}"
               onchange="guardSetValue('${p.id}', this)">${escapeHtml(String(p.value ?? ""))}</textarea>`;
      } else {
        editor = `<span class="guard-default-hint">无可编辑值；启用即执行「${p.applyMode === "toml_absent" ? "删除" : "写入"}」</span>`;
      }
      const meta = p.locked
        ? `<div class="guard-param-meta">上次校验 ${fmtTs(p.lastChecked)} ｜ 上次自动恢复 ${fmtTs(p.lastRestored)}</div>`
        : "";
      return `<div class="guard-param">
        <div class="guard-param-head">
          <span class="guard-param-label">${escapeHtml(p.label)}</span>
          <span style="display:flex;align-items:center;gap:8px;">
            ${p.custom ? `<button class="guard-param-delete" onclick="guardRemoveCustom('${p.id}')" title="删除自定义参数">删除</button>` : ""}
            <span class="status-badge ${s.cls}"><span class="dot"></span><span>${s.text}</span></span>
          </span>
        </div>
        <div class="guard-param-desc">${escapeHtml(p.description)}</div>
        ${p.path ? `<div class="guard-param-path mono">${escapeHtml(p.path)}</div>` : ""}
        <div class="guard-param-actual ${p.status === "match" ? "ok" : "bad"}">
          当前：${escapeHtml(p.actual ?? p.error ?? "未知")}
        </div>
        ${editor}
        <div class="guard-param-controls" style="margin-top: 8px;">
          <button class="btn btn-primary btn-sm" ${p.locked ? "disabled" : ""}
                  onclick="guardApply('${p.id}')">启用</button>
          ${p.locked
            ? `<button class="btn btn-secondary btn-sm" onclick="guardSetLocked('${p.id}', false)">解锁</button>`
            : `<button class="btn btn-secondary btn-sm" ${p.applied ? "" : "disabled"}
                  onclick="guardSetLocked('${p.id}', true)">锁定</button>`}
        </div>
        ${meta}
      </div>`;
    }).join("");
    const addBtn = `<div class="guard-group-add">
      <button onclick="openGuardAddFormFor('${g.id}', '${escapeHtml(g.name)}')">＋ 添加参数</button>
    </div>`;
    return `<div class="guard-group" data-group-id="${g.id}">
      <div class="guard-group-name">${escapeHtml(g.name)}</div>
      <div class="guard-group-file mono">~/.codex/${escapeHtml(g.file)}</div>
      ${g.error ? `<div class="guard-group-error">${escapeHtml(g.error)}</div>` : ""}
      ${params}
      ${addBtn}
    </div>`;
  }).join("");
}

async function guardToggleBool(id: string): Promise<void> {
  const st = guardState.params[id];
  if (st?.locked) return;
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    await invoke("guard_set_value", { id, value: p.value !== true });
    await refreshGuardView(true);
  } catch (e) {
    toast(`修改失败: ${e}`, "error");
  }
}

async function guardSetValue(id: string, input: HTMLInputElement | HTMLTextAreaElement): Promise<void> {
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    const value = p.valueType === "int" ? parseInt(input.value, 10) : input.value;
    if (p.valueType === "int" && Number.isNaN(value)) {
      toast("请输入整数", "error");
      await refreshGuardView(true);
      return;
    }
    await invoke("guard_set_value", { id, value });
    await refreshGuardView(true);
  } catch (e) {
    toast(`保存失败: ${e}`, "error");
    await refreshGuardView(true);
  }
}

async function guardApply(id: string): Promise<void> {
  try {
    await invoke("guard_apply", { id });
    toast("已启用", "success");
  } catch (e) {
    toast(`启用失败: ${e}`, "error");
  }
  await refreshGuardView(true);
}

async function guardSetLocked(id: string, locked: boolean): Promise<void> {
  try {
    await invoke("guard_set_locked", { id, locked });
    toast(locked ? "已锁定" : "已解锁", locked ? "success" : "info");
  } catch (e) {
    toast(`操作失败: ${e}`, "error");
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
      container.textContent = "暂无文件";
    } else {
      const html = guardFiles.map((f) => {
        const delBtn = f.builtin
          ? `<button class="guard-file-btn" disabled style="opacity:0.4;">内置</button>`
          : `<button class="guard-file-btn danger" onclick="guardRemoveFile('${f.id}')">删除</button>`;
        const detectBtn = f.builtin
          ? `<button class="guard-file-btn" onclick="guardDetectFile('${f.id}')">检测</button>`
          : "";
        const det = f.detection;
        const detText = det
          ? det.path === null
            ? `检测记录：未找到该文件（${fmtTs(det.at)}）`
            : det.path === f.file
              ? `检测记录：路径一致（${fmtTs(det.at)}）`
              : `检测记录：实际位于 ${det.path}（${fmtTs(det.at)}）`
          : "";
        return `<div class="guard-file-card" data-file-id="${f.id}">
          <div class="guard-file-card-head">
            <span class="guard-file-name">${escapeHtml(f.name)}</span>
            <span class="guard-file-format">${f.format}</span>
          </div>
          <div class="guard-file-path">~/.codex/${escapeHtml(f.file)}</div>
          ${detText ? `<div class="guard-file-detect">${escapeHtml(detText)}</div>` : ""}
          <div class="guard-file-actions">
            ${detectBtn}
            <button class="guard-file-btn" onclick="guardEditFile('${f.id}')">编辑</button>
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
  if (submit) submit.textContent = "添加";
  const title = document.getElementById("guard-file-modal-title");
  if (title) title.textContent = "添加看守文件";
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
  document.getElementById("settings-guard-file-submit")!.textContent = "保存";
  document.getElementById("guard-file-modal-title")!.textContent = "编辑看守文件";
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
  if (!name) { toast("请填写文件名称", "error"); return; }
  if (!file) { toast("请填写文件路径", "error"); return; }
  try {
    if (editingFileId) {
      await invoke("guard_update_file", { id: editingFileId, name, file });
      toast("已更新", "success");
    } else {
      await invoke("guard_add_file", { name, file, format });
      toast("已添加文件", "success");
    }
    (document.getElementById("settings-guard-file-name") as HTMLInputElement).value = "";
    (document.getElementById("settings-guard-file-path") as HTMLInputElement).value = "";
    toggleGuardFileForm();
    await refreshGuardFiles();
    await refreshGuardView(true);
  } catch (e) {
    toast(`${editingFileId ? "更新" : "添加"}失败: ${e}`, "error");
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
        `检测到「${updated.name}」实际位于：\n~/.codex/${detected}\n\n与当前配置 ~/.codex/${updated.file} 不同，是否更新为检测到的路径？`,
        { title: "更新看守路径", kind: "warning" }
      );
      if (ok) {
        await invoke("guard_update_file", { id, name: updated.name, file: detected });
        toast("已更新为检测到的路径", "success");
        await refreshGuardFiles();
        await refreshGuardView(true);
      }
    } else if (!auto) {
      toast(detected ? "检测完成：路径一致" : "未在 ~/.codex 下找到该文件", detected ? "success" : "info");
    }
  } catch (e) {
    if (!auto) toast(`检测失败: ${e}`, "error");
  }
}

async function guardRemoveFile(id: string): Promise<void> {
  const f = guardFiles.find((x) => x.id === id);
  if (!f) return;
  if (!(await ask(`确定删除文件「${f.name}」？\n\n该文件下的所有自定义参数会被移除看守，但已写入 ~/.codex/${f.file} 的值不会被回滚。`, { title: "删除看守文件", kind: "warning" }))) {
    return;
  }
  try {
    await invoke("guard_remove_file", { id });
    toast("已删除", "success");
    await refreshGuardFiles();
    await refreshGuardView(true);
  } catch (e) {
    toast(`删除失败: ${e}`, "error");
  }
}

// ============ 自定义参数管理 ============
function openGuardAddFormFor(fileId: string): void {
  guardAddParamFileId = fileId;
  const form = document.getElementById("guard-add-form")!;
  form.classList.remove("hidden");
  const fileSelect = document.getElementById("guard-add-file-select") as HTMLSelectElement | null;
  if (fileSelect) {
    fileSelect.value = fileId;
  }
  form.scrollIntoView({ behavior: "smooth", block: "center" });
}

function toggleGuardAddForm(): void {
  const form = document.getElementById("guard-add-form")!;
  const arrow = document.getElementById("guard-add-arrow")!;
  form.classList.toggle("hidden");
  arrow.textContent = form.classList.contains("hidden") ? "▾" : "▴";
  if (!form.classList.contains("hidden")) {
    onGuardAddModeChange();
    onGuardAddValueTypeChange();
  }
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
    inp.className = "guard-form-input";
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
      if (Number.isNaN(n)) throw new Error("默认值必须是整数");
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

  if (!id) { toast("请填写 ID", "error"); return; }
  if (!label) { toast("请填写名称", "error"); return; }
  if (!fileId) { toast("请选择目标文件", "error"); return; }
  if ((mode === "toml_key" || mode === "toml_absent") && !path) {
    toast("请填写 TOML 路径", "error"); return;
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
    toast("已添加自定义参数", "success");
    // 清空表单并收起
    (document.getElementById("guard-add-id") as HTMLInputElement).value = "";
    (document.getElementById("guard-add-label") as HTMLInputElement).value = "";
    (document.getElementById("guard-add-path") as HTMLInputElement).value = "";
    (document.getElementById("guard-add-desc") as HTMLInputElement).value = "";
    defaultEl.value = "";
    guardAddParamFileId = null;
    toggleGuardAddForm();
    await refreshGuardView(true);
  } catch (e) {
    toast(`添加失败: ${e}`, "error");
  }
}

async function guardRemoveCustom(id: string): Promise<void> {
  if (!(await ask(`确定删除自定义参数 ${id}？\n\n删除后看守停止，已写入 ~/.codex/ 的值不会被回滚，可从 ~/.codex/dashi-backups/ 手动恢复。`, { title: "删除自定义参数", kind: "warning" }))) {
    return;
  }
  try {
    await invoke("guard_remove_custom_param", { id });
    toast("已删除", "success");
    await refreshGuardView(true);
  } catch (e) {
    toast(`删除失败: ${e}`, "error");
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
      toast(`路径已复制到剪贴板: ${path}`, "info");
    } catch {
      toast(`打开失败: ${e}`, "error");
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
      el.textContent = "已就绪";
      el.className = "health-status ok";
      helpRow.classList.add("hidden");
    } else {
      el.textContent = health.message;
      el.className = "health-status err";
      helpRow.classList.remove("hidden");
    }
  } catch (e) {
    el.textContent = `检查失败: ${e}`;
    el.className = "health-status err";
    helpRow.classList.remove("hidden");
  }
}

async function openUpdaterHelp(target: "docs" | "template"): Promise<void> {
  try {
    const paths = await invoke<UpdaterHelpPaths>("get_updater_help_paths");
    await openUrl(target === "docs" ? paths.docsPath : paths.templatePath);
  } catch (e) {
    toast(`打开帮助失败: ${e}`, "error");
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
    btn.textContent = "立即更新";
  } else {
    row.classList.add("hidden");
    btn.textContent = "检查更新";
  }
}

function renderDownloadProgress(p: DownloadProgress): void {
  const row = document.getElementById("update-progress-row")!;
  row.classList.remove("hidden");
  const bar = document.getElementById("update-progress-bar")!;
  const text = document.getElementById("update-progress-text")!;
  if (p.stage === "restarting") {
    bar.style.width = "100%";
    text.textContent = "安装完成，正在重启…";
  } else if (p.stage === "installing") {
    bar.style.width = "100%";
    text.textContent = "正在安装…";
  } else if (p.stage === "retrying") {
    text.textContent = `下载失败，正在重试（${p.attempt}/${p.maxAttempts}）…`;
  } else {
    if (p.percent !== null) {
      bar.style.width = `${p.percent}%`;
      text.textContent = `正在下载 v${p.version}：${Math.floor(p.percent)}%`;
    } else {
      const mb = (p.downloadedBytes / 1024 / 1024).toFixed(1);
      text.textContent = `正在下载 v${p.version}：${mb} MB`;
    }
  }
}

async function checkUpdate(silent = false): Promise<void> {
  if (updateBusy) return;
  updateBusy = true;
  const btn = document.getElementById("btn-check-update")! as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = "检查中...";
  try {
    const info = await invoke<UpdateInfo>("check_update");
    renderUpdateInfo(info);
    if (info.hasUpdate) {
      toast(`发现新版本: v${info.availableVersion}`, "info");
    } else if (info.message) {
      if (!silent) toast(info.message, "error");
    } else if (!silent) {
      toast("当前已是最新版本", "info");
    }
  } catch (e) {
    if (!silent) toast(`检查更新失败: ${e}`, "error");
  } finally {
    updateBusy = false;
    btn.disabled = false;
    if (!pendingUpdateInfo) btn.textContent = "检查更新";
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
  btn.textContent = "更新中...";
  try {
    const msg = await invoke<string>("install_update", {
      expectedVersion: pendingUpdateInfo.availableVersion,
    });
    toast(msg, "success");
    pendingUpdateInfo = null;
    document.getElementById("update-available-row")!.classList.add("hidden");
    btn.textContent = "检查更新";
  } catch (e) {
    toast(`更新失败: ${e}`, "error");
    btn.textContent = "立即更新";
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
    toast(`打开链接失败: ${e}`, "error");
  }
}

// ============ 状态更新 ============
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

  const badge = document.getElementById(`badge-${prefix}`)!;
  const msg = document.getElementById(`msg-${prefix}`)!;
  const statusText = document.getElementById(`badge-${prefix}-text`)!;

  const statusMap: Record<string, { text: string; cls: string }> = {
    running: { text: "运行中", cls: "running" },
    stopped: { text: "已停止", cls: "stopped" },
    starting: { text: "启动中", cls: "starting" },
    stopping: { text: "停止中", cls: "stopping" },
    failed: { text: "失败", cls: "failed" },
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
async function init(): Promise<void> {
  // 应用主题
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
    toast(`初始化失败: ${e}`, "error");
  }
}

// ============ 暴露到全局 ============
const w = window as unknown as Record<string, unknown>;
w.toggleSettings = toggleSettings;
w.showHome = showHome;
w.showSkill = showSkill;
w.setTheme = setTheme;
w.toggleTrayMinimize = toggleTrayMinimize;
w.switchSection = switchSection;
w.browsePath = browsePath;
w.browseNode = browseNode;
w.browseCodex = browseCodex;
w.useBundledTaskboard = useBundledTaskboard;
w.toggleMode = toggleMode;
w.toggleAutoOpen = toggleAutoOpen;
w.onConfigChange = onConfigChange;
w.saveConfig = saveConfig;
w.startAll = startAll;
w.stopAll = stopAll;
w.startTaskboard = startTaskboard;
w.stopTaskboard = stopTaskboard;
w.startInjector = startInjector;
w.stopInjector = stopInjector;
w.openTaskboard = openTaskboard;
w.installSkill = installSkill;
w.showGuard = showGuard;
w.toggleGuard = toggleGuard;
w.guardToggleBool = guardToggleBool;
w.guardSetValue = guardSetValue;
w.guardApply = guardApply;
w.guardSetLocked = guardSetLocked;
w.toggleGuardAddForm = toggleGuardAddForm;
w.onGuardAddModeChange = onGuardAddModeChange;
w.onGuardAddValueTypeChange = onGuardAddValueTypeChange;
w.guardAddCustom = guardAddCustom;
w.guardRemoveCustom = guardRemoveCustom;
w.guardOpenSchemaFile = guardOpenSchemaFile;
w.toggleGuardFileForm = toggleGuardFileForm;
w.guardSaveFileForm = guardSaveFileForm;
w.guardEditFile = guardEditFile;
w.guardDetectFile = guardDetectFile;
w.guardPickFilePath = guardPickFilePath;
w.guardRemoveFile = guardRemoveFile;
w.openGuardAddFormFor = openGuardAddFormFor;
w.checkUpdate = onUpdateButton;
w.openUpdaterHelp = openUpdaterHelp;
w.openGithub = openGithub;
w.toggleFastctx = toggleFastctx;
w.openFastctxConsole = openFastctxConsole;

init();
