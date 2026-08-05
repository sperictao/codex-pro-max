import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
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
    await invoke("save_config", { config: cfg });
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
  const selected = await openDialog({ directory: true, multiple: false });
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
    document.getElementById("btn-skill")!.classList.remove("active");
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
  document.getElementById("btn-settings")!.classList.remove("active");
  document.getElementById("btn-skill")!.classList.remove("active");
  document.getElementById("btn-home")!.classList.add("active");
}

function showSkill(): void {
  document.getElementById("main-view")!.classList.add("hidden");
  document.getElementById("settings-view")!.classList.add("hidden");
  document.getElementById("skill-view")!.classList.remove("hidden");
  document.getElementById("btn-settings")!.classList.remove("active");
  document.getElementById("btn-home")!.classList.remove("active");
  document.getElementById("btn-skill")!.classList.add("active");
  void refreshSkillStatus();
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
  if (section === "about" || section === "appearance") {
    footer.classList.add("hidden");
  } else {
    footer.classList.remove("hidden");
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
    await invoke("save_config", { config: cfg });
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
        await invoke("save_config", { config: readConfigFromUI() });
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
    statusPolling = setInterval(refreshStatus, 3000);
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
w.checkUpdate = onUpdateButton;
w.openUpdaterHelp = openUpdaterHelp;
w.openGithub = openGithub;

init();
