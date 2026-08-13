import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { initI18n, applyDomTranslations, applyLanguage, currentLanguage, t } from "./i18n";
import type { ResolvedLanguage } from "./i18n";
import type { ThemeMode } from "./theme";
import { state } from "./state";
import type { LauncherConfig } from "./state";
import {
  toast,
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
import { toggleSettings, showHome, showSkill, showGuard, showIntegration, switchSection } from "./nav";
import {
  startAll,
  stopAll,
  startTaskboard,
  stopTaskboard,
  startInjector,
  stopInjector,
  openTaskboard,
  useBundledTaskboard,
  browsePath,
  browseNode,
  browseCodex,
  refreshSkillStatus,
  installSkill,
  refreshStatus,
  updateStatusUI,
} from "./service";
import type { ProcessInfo } from "./service";
import {
  toggleGuard,
  renderGuardToggle,
  refreshGuardView,
  renderGuardFiles,
  refreshGuardFiles,
  toggleGuardFileForm,
  guardEditFile,
  guardPickFilePath,
  guardSaveFileForm,
  guardDetectFile,
  guardRemoveFile,
  openGuardAddFormFor,
  openGuardAddModal,
  closeGuardAddModal,
  onGuardAddModeChange,
  onGuardAddValueTypeChange,
  guardAddCustom,
  guardRemoveCustom,
  guardOpenSchemaFile,
  guardApply,
  guardDisable,
  guardSetLocked,
  guardToggleBool,
  guardToggleApplied,
  guardSetValue,
} from "./guard";
import { renderFastctx, refreshFastctxStatus, toggleFastctx, openFastctxConsole } from "./fastctx";
import {
  renderDsh,
  refreshDshStatus,
  startDshRemote,
  stopDshRemote,
  openDshRemote,
  toggleDshAutostart,
  bindDshEvents,
} from "./dsh";
import {
  checkUpdaterHealth,
  openUpdaterHelp,
  renderUpdateInfo,
  renderDownloadProgress,
  checkUpdate,
  onUpdateButton,
  openGithub,
  pendingUpdateInfo,
} from "./updater";
import type { DownloadProgress } from "./updater";

// shell：跨域编排（语言切换/配置填充/初始化/事件接线），唯一允许 import 所有域的模块（ADR 0009）

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
  renderDsh();
  renderGuardFiles();
  renderUpdateInfo(pendingUpdateInfo ?? {
    currentVersion: "", availableVersion: null, hasUpdate: false, releaseNotes: null, message: null,
  });
  void refreshStatus();
  void refreshGuardView(true);
  void validatePaths();
  void refreshSkillStatus();
}

// ============ 配置填充（壳侧编排；读写与校验在 core.ts） ============
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
  await bindDshEvents();
}

// ============ 初始化 ============
let statusPolling: ReturnType<typeof setInterval> | null = null;

export async function init(): Promise<void> {
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
    void refreshDshStatus();

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
  on("btn-integration", "click", () => {
    showIntegration();
    void refreshFastctxStatus();
    void refreshDshStatus();
  });

  // 主页
  on("btn-start-all", "click", () => void startAll());
  on("btn-stop-all", "click", () => void stopAll());
  on("btn-start-tb", "click", () => void startTaskboard());
  on("btn-stop-tb", "click", () => void stopTaskboard());
  on("btn-open-tb", "click", () => void openTaskboard());
  on("btn-start-inj", "click", () => void startInjector());
  on("btn-stop-inj", "click", () => void stopInjector());

  // 设置侧栏（guard 分区的数据刷新在调用点触发）
  for (const s of ["general", "appearance", "network", "mode", "guard", "about"]) {
    on(`nav-${s}`, "click", () => {
      switchSection(s);
      if (s === "guard") void refreshGuardFiles();
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

  // dsh 远程访问
  on("btn-dsh-start", "click", () => void startDshRemote());
  on("btn-dsh-stop", "click", () => void stopDshRemote());
  on("btn-dsh-open", "click", () => void openDshRemote());
  on("toggle-dsh-autostart", "change", () => void toggleDshAutostart());

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
