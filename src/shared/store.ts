// shared/store：全局状态（Zustand）。config 是设置页草稿（沿用旧 readConfigFromUI 语义：
// 输入即改草稿，Save/启动时才落盘）。Tauri 推送事件经事件桥直写本 store。

import { create } from "zustand";
import { getStoredFamily, getStoredTheme, resolveDataTheme, type ThemeMode } from "./theme";
import { currentLanguage, i18n } from "./i18n";
import * as cmd from "./commands";
import { currentConfigDraft } from "./config";
import type {
  DownloadProgress,
  DshStepEvent,
  GuardFileView,
  GuardState,
  GuardView,
  LauncherConfig,
  ProcessInfo,
  UpdateInfo,
  UpdaterConfigHealth,
} from "./types";

export type View = "home" | "skill" | "guard" | "integration" | "settings";
export type SettingsSection = "general" | "appearance" | "network" | "mode" | "guard" | "about";
export type ToastType = "success" | "error" | "info";

export interface ToastItem {
  id: string;
  message: string;
  type: ToastType;
}

const initialServices = (): { taskboard: ProcessInfo; injector: ProcessInfo } => ({
  taskboard: { name: "taskboard-server", status: "stopped", pid: null, message: "" },
  injector: { name: "injector", status: "stopped", pid: null, message: "" },
});

function applyDataTheme(mode: ThemeMode, family: string): void {
  document.documentElement.dataset.theme = resolveDataTheme(
    mode,
    family,
    window.matchMedia("(prefers-color-scheme: dark)").matches,
  );
}

// 看守视图增量刷新基线（旧 lastGuardJson）：内容未变的轮询不触发重渲染
let lastGuardJson = "";

// 模块求值时机不保证 DOM 全局就绪（vitest 4 模块执行器在被依赖模块求值后才装 jsdom 全局），
// 读 localStorage 一律走这里：非 DOM 上下文回落 null（= 默认主题）
function readStored(key: string): string | null {
  return typeof localStorage === "undefined" ? null : localStorage.getItem(key);
}

interface AppStore {
  // 导航
  activeView: View;
  settingsSection: SettingsSection;
  // 配置草稿与看守状态
  config: LauncherConfig | null;
  guardState: GuardState;
  autostart: boolean;
  languageSetting: string;
  appVersion: string;
  // 进程状态（received 标记是否收到过首次状态：未收到时消息行显示 "Not started"，
  // 收到后空消息显示 "-"——复刻旧静态 HTML 初始文案与 updateStatusUI 的差异）
  services: { taskboard: ProcessInfo; injector: ProcessInfo };
  servicesReceived: { taskboard: boolean; injector: boolean };
  // 事件桥写入区
  dshTimeline: DshStepEvent[];
  downloadProgress: DownloadProgress | null;
  // 看守（视图数据 + 文件列表；操作逻辑在 features/guard/ops.ts）
  guardView: GuardView | null;
  guardFiles: GuardFileView[];
  // 更新器（updateInfo 即旧 pendingUpdateInfo：仅有可用更新时非空）
  updaterHealth: UpdaterConfigHealth | null;
  updaterHealthError: string | null;
  updateInfo: UpdateInfo | null;
  updateBusyKind: "check" | "install" | null;
  // 主题（localStorage 是唯一事实来源，store 是渲染镜像）
  themeMode: ThemeMode;
  themeFamily: string;
  toasts: ToastItem[];

  navigate: (view: View) => void;
  setSettingsSection: (section: SettingsSection) => void;
  toast: (message: string, type?: ToastType) => void;
  dismissToast: (id: string) => void;
  setThemeMode: (mode: ThemeMode) => void;
  setThemeFamily: (family: string) => void;
  syncSystemTheme: () => void;
  applyConfig: (cfg: LauncherConfig) => void;
  setConfigField: (patch: Partial<LauncherConfig>) => void;
  setAutostart: (enabled: boolean) => void;
  updateService: (info: ProcessInfo) => void;
  refreshStatus: () => Promise<void>;
  handleDshStep: (step: DshStepEvent) => void;
  setDshTimeline: (steps: DshStepEvent[]) => void;
  setDownloadProgress: (p: DownloadProgress) => void;
  setLanguageSetting: (setting: string) => Promise<void>;
  saveConfig: () => Promise<void>;
  toggleAutostart: () => Promise<void>;
  toggleGuardEnabled: () => Promise<void>;
  setAppVersion: (v: string) => void;
  refreshUpdaterHealth: () => Promise<void>;
  checkForUpdates: (silent?: boolean) => Promise<void>;
  installPendingUpdate: () => Promise<void>;
  refreshGuardView: (force?: boolean) => Promise<void>;
  setGuardFiles: (files: GuardFileView[]) => void;
}

export const useAppStore = create<AppStore>()((set, get) => ({
  activeView: "home",
  settingsSection: "general",
  config: null,
  guardState: { enabled: false, params: {} },
  autostart: false,
  languageSetting: "system",
  appVersion: "-",
  services: initialServices(),
  servicesReceived: { taskboard: false, injector: false },
  dshTimeline: [],
  downloadProgress: null,
  updaterHealth: null,
  updaterHealthError: null,
  updateInfo: null,
  updateBusyKind: null,
  guardView: null,
  guardFiles: [],
  themeMode: getStoredTheme(readStored("theme")),
  themeFamily: getStoredFamily(readStored("theme-family")),
  toasts: [],

  // 设置/集成是 toggle 语义：已在该视图时再点回主页（旧 nav.ts 行为）
  navigate: (view) => {
    const cur = get().activeView;
    if ((view === "settings" || view === "integration") && cur === view) {
      set({ activeView: "home" });
    } else {
      set({ activeView: view });
    }
  },
  setSettingsSection: (section) => set({ settingsSection: section }),

  toast: (message, type = "info") => {
    const id = crypto.randomUUID();
    set((s) => ({ toasts: [...s.toasts, { id, message, type }] }));
    // 3s 后组件开始淡出，3.3s 后移除（与旧 toast 时序一致）
    setTimeout(() => get().dismissToast(id), 3300);
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  setThemeMode: (mode) => {
    localStorage.setItem("theme", mode);
    applyDataTheme(mode, get().themeFamily);
    set({ themeMode: mode });
  },
  setThemeFamily: (family) => {
    localStorage.setItem("theme-family", family);
    applyDataTheme(get().themeMode, family);
    set({ themeFamily: family });
  },
  syncSystemTheme: () => {
    if (get().themeMode === "system") applyDataTheme("system", get().themeFamily);
  },

  applyConfig: (cfg) =>
    set({
      config: cfg,
      guardState: cfg.codex_guard ?? { enabled: false, params: {} },
      languageSetting: cfg.language || "system",
    }),
  setConfigField: (patch) => set((s) => ({ config: s.config ? { ...s.config, ...patch } : s.config })),
  setAutostart: (enabled) => set({ autostart: enabled }),

  updateService: (info) => {
    const key = info.name === "taskboard-server" ? "taskboard" : "injector";
    set((s) => ({
      services: { ...s.services, [key]: info },
      servicesReceived: { ...s.servicesReceived, [key]: true },
    }));
  },
  // 轮询错误忽略（与旧 refreshStatus 一致）
  refreshStatus: async () => {
    try {
      const list = await cmd.getStatus();
      for (const info of list) get().updateService(info);
    } catch {
      /* 忽略 */
    }
  },

  handleDshStep: (step) =>
    set((s) => {
      const tl = [...s.dshTimeline];
      const i = tl.findIndex((x) => x.index === step.index);
      if (i >= 0) {
        tl[i] = step;
      } else {
        tl.push(step);
        tl.sort((a, b) => a.index - b.index);
      }
      return { dshTimeline: tl };
    }),
  setDshTimeline: (steps) => set({ dshTimeline: steps }),
  setDownloadProgress: (p) => set({ downloadProgress: p }),

  // 语言切换编排（旧 shell.setLanguage）：落盘 + Rust 重建托盘 + react-i18next 响应式重渲染
  setLanguageSetting: async (setting) => {
    set({ languageSetting: setting });
    try {
      const cfg = get().config;
      if (cfg) await cmd.updateSettings({ ...cfg, language: setting, codex_guard: get().guardState });
      await cmd.setLanguage(setting);
      const resolved = await cmd.getResolvedLanguage();
      await i18n.changeLanguage(resolved === "zh-CN" ? "zh-CN" : "en");
      document.documentElement.lang = currentLanguage();
    } catch (e) {
      get().toast(i18n.t("Save failed: {{error}}", { error: String(e) }), "error");
    }
  },

  // 保存设置（旧 core.saveConfig）：update_settings 不动看守状态，保存后回读同步 guardState
  saveConfig: async () => {
    try {
      await cmd.updateSettings(currentConfigDraft(get()));
      const latest = await cmd.loadConfig();
      set({ guardState: latest.codex_guard ?? { enabled: false, params: {} } });
      get().toast(i18n.t("Settings saved"), "success");
    } catch (e) {
      get().toast(i18n.t("Save failed: {{error}}", { error: String(e) }), "error");
    }
  },

  // 自启开关即时写 OS 注册项，失败回退（旧 toggleAutostart）
  toggleAutostart: async () => {
    const next = !get().autostart;
    set({ autostart: next });
    try {
      await cmd.autostartSet(next);
    } catch (e) {
      set({ autostart: !next });
      get().toast(String(e), "error");
    }
  },

  // 看守总开关（旧 guard.toggleGuard；失败时开关状态不变即回退）
  toggleGuardEnabled: async () => {
    const next = !get().guardState.enabled;
    try {
      await cmd.guardSetEnabled(next);
      set((s) => ({ guardState: { ...s.guardState, enabled: next } }));
      get().toast(
        next ? i18n.t("Config guard enabled") : i18n.t("Config guard disabled"),
        next ? "success" : "info",
      );
    } catch (e) {
      get().toast(i18n.t("Toggle failed: {{error}}", { error: String(e) }), "error");
    }
  },

  setAppVersion: (v) => set({ appVersion: v }),

  // 更新源健康（旧 checkUpdaterHealth）
  refreshUpdaterHealth: async () => {
    try {
      set({ updaterHealth: await cmd.getUpdaterConfigHealth(), updaterHealthError: null });
    } catch (e) {
      set({ updaterHealth: null, updaterHealthError: String(e) });
    }
  },

  // 检查更新（旧 checkUpdate；silent 时静默失败/静默无更新）
  checkForUpdates: async (silent = false) => {
    if (get().updateBusyKind) return;
    set({ updateBusyKind: "check" });
    try {
      const info = await cmd.checkUpdate();
      set({ updateInfo: info.hasUpdate ? info : null });
      if (info.hasUpdate) {
        get().toast(i18n.t("New version available: v{{version}}", { version: String(info.availableVersion) }), "info");
      } else if (info.message) {
        if (!silent) get().toast(info.message, "error");
      } else if (!silent) {
        get().toast(i18n.t("Already up to date"), "info");
      }
    } catch (e) {
      if (!silent) get().toast(i18n.t("Failed to check for updates: {{error}}", { error: String(e) }), "error");
    } finally {
      set({ updateBusyKind: null });
    }
  },

  // 无待装更新时退化为检查更新（旧 onUpdateButton）
  installPendingUpdate: async () => {
    const pending = get().updateInfo;
    if (!pending) {
      await get().checkForUpdates();
      return;
    }
    if (get().updateBusyKind) return;
    set({ updateBusyKind: "install" });
    try {
      const msg = await cmd.installUpdate(pending.availableVersion);
      get().toast(msg, "success");
      set({ updateInfo: null });
    } catch (e) {
      get().toast(i18n.t("Update failed: {{error}}", { error: String(e) }), "error");
    } finally {
      // 旧 finally：隐藏进度行并归零进度条
      set({ updateBusyKind: null, downloadProgress: null });
    }
  },

  // 看守视图刷新（旧 refreshGuardView 语义逐条保留）：
  // 视图不在前台时跳过；非强制且内容未变跳过；非强制且焦点在看守视图输入框内跳过（不抢焦点）
  refreshGuardView: async (force = false) => {
    if (get().activeView !== "guard") return;
    try {
      const view = await cmd.guardGetView();
      const json = JSON.stringify(view);
      if (!force && json === lastGuardJson) return;
      if (!force) {
        const ae = document.activeElement;
        if (ae && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA")) {
          const root = document.getElementById("guard-view");
          if (root?.contains(ae)) return;
        }
      }
      lastGuardJson = json;
      set({ guardView: view });
    } catch {
      /* 轮询错误忽略 */
    }
  },
  setGuardFiles: (files) => set({ guardFiles: files }),
}));
