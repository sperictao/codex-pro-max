import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

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
}

interface ProcessInfo {
  name: string;
  status: "stopped" | "starting" | "running" | "stopping" | "failed";
  pid: number | null;
  message: string;
}

// ============ 全局状态 ============
let config: LauncherConfig | null = null;
let statusPolling: ReturnType<typeof setInterval> | null = null;
// eslint-disable-next-line @typescript-eslint/no-unused-vars
void statusPolling;

// ============ Toast 通知 ============
function toast(message: string, type: "success" | "error" | "info" = "info") {
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

// ============ 配置管理 ============
async function loadConfig(): Promise<LauncherConfig> {
  const cfg = await invoke<LauncherConfig>("load_config");
  config = cfg;
  return cfg;
}

async function saveConfig() {
  if (!config) return;
  try {
    await invoke("save_config", { config });
    toast("配置已保存", "success");
  } catch (e) {
    toast(`保存失败: ${e}`, "error");
  }
}

function fillConfigUI(cfg: LauncherConfig) {
  (document.getElementById("cfg-path") as HTMLInputElement).value = cfg.taskboard_path;
  (document.getElementById("cfg-node") as HTMLInputElement).value = cfg.node_path;
  (document.getElementById("cfg-codex") as HTMLInputElement).value = cfg.codex_app_path;
  (document.getElementById("cfg-port") as HTMLInputElement).value = String(cfg.taskboard_port);
  (document.getElementById("cfg-cdp") as HTMLInputElement).value = String(cfg.cdp_port);

  const toggle = document.getElementById("toggle-mode")!;
  if (cfg.separate_window_mode) {
    toggle.classList.add("active");
  } else {
    toggle.classList.remove("active");
  }
  updateToggleLabel();
}

function readConfigFromUI(): LauncherConfig {
  return {
    taskboard_path: (document.getElementById("cfg-path") as HTMLInputElement).value,
    node_path: (document.getElementById("cfg-node") as HTMLInputElement).value,
    codex_app_path: (document.getElementById("cfg-codex") as HTMLInputElement).value,
    taskboard_port: parseInt((document.getElementById("cfg-port") as HTMLInputElement).value) || 47823,
    taskboard_host: "127.0.0.1",
    cdp_port: parseInt((document.getElementById("cfg-cdp") as HTMLInputElement).value) || 9231,
    auto_open: true,
    separate_window_mode: document.getElementById("toggle-mode")!.classList.contains("active"),
  };
}

function onConfigChange() {
  if (!config) return;
  config = readConfigFromUI();
  validatePaths();
}

async function validatePaths() {
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
  }

  // 验证 node
  const nodeEl = document.getElementById("validate-node")!;
  if (cfg.node_path || true) {
    try {
      const version = await invoke<string>("check_node_version", { nodePath: cfg.node_path });
      nodeEl.textContent = version;
      nodeEl.className = "config-validate ok";
    } catch {
      nodeEl.textContent = "不可用";
      nodeEl.className = "config-validate err";
    }
  }

  // 验证 codex app
  const codexEl = document.getElementById("validate-codex")!;
  if (cfg.codex_app_path) {
    try {
      const exists = await invoke<boolean>("check_codex_app", { appPath: cfg.codex_app_path });
      codexEl.textContent = exists ? "存在" : "不存在";
      codexEl.className = `config-validate ${exists ? "ok" : "err"}`;
    } catch {
      codexEl.textContent = "";
    }
  }
}

// ============ 文件浏览 ============
async function browsePath() {
  const selected = await openDialog({ directory: true, multiple: false });
  if (selected) {
    (document.getElementById("cfg-path") as HTMLInputElement).value = selected as string;
    onConfigChange();
  }
}

async function browseNode() {
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

async function browseCodex() {
  const selected = await openDialog({ directory: true, multiple: false });
  if (selected) {
    (document.getElementById("cfg-codex") as HTMLInputElement).value = selected as string;
    onConfigChange();
  }
}

// ============ 模式切换 ============
function toggleMode() {
  const toggle = document.getElementById("toggle-mode")!;
  toggle.classList.toggle("active");
  updateToggleLabel();
  onConfigChange();
}

function updateToggleLabel() {
  const toggle = document.getElementById("toggle-mode")!;
  const label = document.getElementById("toggle-mode-label")!;
  if (toggle.classList.contains("active")) {
    label.textContent = "独立窗口模式（不重启 Codex）";
  } else {
    label.textContent = "完整启动模式（重启 Codex）";
  }
}

// ============ 面板切换 ============
function toggleSettings() {
  document.getElementById("settings-panel")!.classList.toggle("settings-hidden");
  document.getElementById("skill-panel")!.classList.add("settings-hidden");
  const btn = document.getElementById("btn-settings")!;
  btn.classList.toggle("active");
  document.getElementById("btn-skill")!.classList.remove("active");
}

function toggleSkill() {
  document.getElementById("skill-panel")!.classList.toggle("settings-hidden");
  document.getElementById("settings-panel")!.classList.add("settings-hidden");
  const btn = document.getElementById("btn-skill")!;
  btn.classList.toggle("active");
  document.getElementById("btn-settings")!.classList.remove("active");
}

// ============ 启动/停止 ============
async function startAll() {
  const cfg = readConfigFromUI();
  if (!cfg.taskboard_path) {
    toast("请先在设置中配置 Taskboard 路径", "error");
    toggleSettings();
    return;
  }

  const btn = document.getElementById("btn-start-all")! as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = "启动中...";

  try {
    // 先保存配置
    config = cfg;
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

async function stopAll() {
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

async function startTaskboard() {
  const cfg = readConfigFromUI();
  try {
    await invoke("start_taskboard", { config: cfg });
    toast("Taskboard 服务已启动", "success");
    await refreshStatus();
  } catch (e) {
    toast(`启动失败: ${e}`, "error");
  }
}

async function stopTaskboard() {
  try {
    await invoke("stop_taskboard");
    toast("Taskboard 服务已停止", "info");
    await refreshStatus();
  } catch (e) {
    toast(`停止失败: ${e}`, "error");
  }
}

async function startInjector() {
  const cfg = readConfigFromUI();
  try {
    await invoke("start_injector", { config: cfg });
    toast("Codex 注入器已启动", "success");
    await refreshStatus();
  } catch (e) {
    toast(`启动失败: ${e}`, "error");
  }
}

async function stopInjector() {
  try {
    await invoke("stop_injector");
    toast("Codex 注入器已停止", "info");
    await refreshStatus();
  } catch (e) {
    toast(`停止失败: ${e}`, "error");
  }
}

async function openTaskboard() {
  const cfg = readConfigFromUI();
  try {
    await invoke("open_taskboard", { config: cfg });
  } catch (e) {
    toast(`打开失败: ${e}`, "error");
  }
}

// ============ Skill 安装 ============
async function installSkill() {
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
}

// ============ 状态更新 ============
async function refreshStatus() {
  try {
    const statuses = await invoke<ProcessInfo[]>("get_status");
    for (const s of statuses) {
      updateStatusUI(s);
    }
    updateGlobalButtons(statuses);
  } catch {
    // 忽略
  }
}

function updateStatusUI(info: ProcessInfo) {
  const isTaskboard = info.name === "taskboard-server";
  const prefix = isTaskboard ? "taskboard" : "injector";

  const badge = document.getElementById(`badge-${prefix}`)!;
  const msg = document.getElementById(`msg-${prefix}`)!;
  const statusText = badge.querySelector("span:last-child")!;

  const statusMap: Record<string, { text: string; class: string }> = {
    running: { text: "运行中", class: "running" },
    stopped: { text: "已停止", class: "stopped" },
    starting: { text: "启动中", class: "starting" },
    stopping: { text: "停止中", class: "stopping" },
    failed: { text: "失败", class: "failed" },
  };

  const s = statusMap[info.status] || statusMap.stopped;
  badge.className = `status-badge ${s.class}`;
  statusText.textContent = s.text;
  msg.textContent = info.message || "—";

  // 更新按钮状态
  const startBtn = document.getElementById(`btn-start-${isTaskboard ? "tb" : "inj"}`)! as HTMLButtonElement;
  const stopBtn = document.getElementById(`btn-stop-${isTaskboard ? "tb" : "inj"}`)! as HTMLButtonElement;

  startBtn.disabled = info.status === "running" || info.status === "starting";
  stopBtn.disabled = info.status !== "running";

  if (isTaskboard) {
    const openBtn = document.getElementById("btn-open-tb")! as HTMLButtonElement;
    openBtn.disabled = info.status !== "running";
  }
}

function updateGlobalButtons(statuses: ProcessInfo[]) {
  const anyRunning = statuses.some(s => s.status === "running" || s.status === "starting");
  const allStopped = statuses.every(s => s.status === "stopped" || s.status === "failed");

  (document.getElementById("btn-start-all")! as HTMLButtonElement).disabled = anyRunning;
  (document.getElementById("btn-stop-all")! as HTMLButtonElement).disabled = allStopped;
}

// ============ 事件监听 ============
async function setupEventListener() {
  await listen("status-update", (event: any) => {
    const payload = event.payload as { name: string; status: string; message: string };
    const info: ProcessInfo = {
      name: payload.name,
      status: payload.status as ProcessInfo["status"],
      pid: null,
      message: payload.message,
    };
    updateStatusUI(info);
  });
}

// ============ 初始化 ============
async function init() {
  try {
    const cfg = await loadConfig();
    fillConfigUI(cfg);
    await validatePaths();
    await setupEventListener();
    await refreshStatus();

    // 启动状态轮询（每 3 秒）
    statusPolling = setInterval(refreshStatus, 3000);
  } catch (e) {
    toast(`初始化失败: ${e}`, "error");
  }
}

// 暴露到全局
(window as any).toggleSettings = toggleSettings;
(window as any).toggleSkill = toggleSkill;
(window as any).browsePath = browsePath;
(window as any).browseNode = browseNode;
(window as any).browseCodex = browseCodex;
(window as any).toggleMode = toggleMode;
(window as any).onConfigChange = onConfigChange;
(window as any).saveConfig = saveConfig;
(window as any).startAll = startAll;
(window as any).stopAll = stopAll;
(window as any).startTaskboard = startTaskboard;
(window as any).stopTaskboard = stopTaskboard;
(window as any).startInjector = startInjector;
(window as any).stopInjector = stopInjector;
(window as any).openTaskboard = openTaskboard;
(window as any).installSkill = installSkill;

init();
