// shared/store：全局状态（Zustand）。config 是设置页草稿（沿用旧 readConfigFromUI 语义：
// 输入即改草稿，Save/启动时才落盘）。Tauri 推送事件经事件桥直写本 store。

import { create } from "zustand";
import { getStoredFamily, getStoredTheme, resolveDataTheme, type ThemeMode } from "@/theme";
import { currentLanguage, i18n } from "./i18n";
import * as cmd from "./commands";
import type { DownloadProgress, DshStepEvent, GuardState, LauncherConfig, ProcessInfo } from "./types";

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

interface AppStore {
  // 导航
  activeView: View;
  settingsSection: SettingsSection;
  // 配置草稿与看守状态
  config: LauncherConfig | null;
  guardState: GuardState;
  autostart: boolean;
  languageSetting: string;
  // 进程状态
  services: { taskboard: ProcessInfo; injector: ProcessInfo };
  // 事件桥写入区
  dshTimeline: DshStepEvent[];
  downloadProgress: DownloadProgress | null;
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
  setDownloadProgress: (p: DownloadProgress) => void;
  setLanguageSetting: (setting: string) => Promise<void>;
}

export const useAppStore = create<AppStore>()((set, get) => ({
  activeView: "home",
  settingsSection: "general",
  config: null,
  guardState: { enabled: false, params: {} },
  autostart: false,
  languageSetting: "system",
  services: initialServices(),
  dshTimeline: [],
  downloadProgress: null,
  themeMode: getStoredTheme(localStorage.getItem("theme")),
  themeFamily: getStoredFamily(localStorage.getItem("theme-family")),
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
    set((s) => ({ services: { ...s.services, [key]: info } }));
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
}));
