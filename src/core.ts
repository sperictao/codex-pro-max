// core：toast、HTML 转义、时间格式化、主题管理、语言卡片、设置页配置读写与路径校验
// 工具域：不 import 任何其他域模块（ADR 0009）

import { invoke } from "@tauri-apps/api/core";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { t, currentLanguage } from "./i18n";
import { THEME_FAMILIES } from "./theme-families";
import {
  getStoredFamily as resolveStoredFamily,
  getStoredTheme as resolveStoredTheme,
  resolveDataTheme as resolveThemeData,
} from "./theme";
import type { ThemeMode } from "./theme";
import { state } from "./state";
import type { LauncherConfig } from "./state";

// ============ Toast 通知 ============
export function toast(message: string, type: "success" | "error" | "info" = "info"): void {
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

export function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

export function fmtTs(ts: number | null): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString(currentLanguage() === "zh-CN" ? "zh-CN" : "en-US", { hour12: false });
}

// ============ 主题管理（ADR 0008：tweakcn token 族 × 模式 二维模型） ============
// 选择器只列亮族；暗面由命名约定 <族id>-dark 决定。默认族 vercel 是视觉基准

const CHECK_SVG = `<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>`;

export function getStoredTheme(): ThemeMode {
  return resolveStoredTheme(localStorage.getItem("theme"));
}

function getStoredFamily(): string {
  return resolveStoredFamily(localStorage.getItem("theme-family"));
}

function resolveDataTheme(mode: ThemeMode, family: string): string {
  return resolveThemeData(
    mode,
    family,
    window.matchMedia("(prefers-color-scheme: dark)").matches,
  );
}

export function applyTheme(mode: ThemeMode): void {
  document.documentElement.dataset.theme = resolveDataTheme(mode, getStoredFamily());

  for (const m of ["light", "dark", "system"] as ThemeMode[]) {
    const card = document.getElementById(`theme-card-${m}`);
    card?.classList.toggle("selected", m === mode);
    card?.setAttribute("aria-pressed", String(m === mode));
  }
  renderThemeFamilySelection();
}

export function setTheme(mode: ThemeMode): void {
  localStorage.setItem("theme", mode);
  applyTheme(mode);
}

export function setThemeFamily(family: string): void {
  localStorage.setItem("theme-family", family);
  applyTheme(getStoredTheme());
}

// 色板卡：卡面自身 data-theme 局部生效，直接渲染该族亮主题（内容来自构建期 manifest，非用户输入）
export function renderThemeFamilyGrid(): void {
  const grid = document.getElementById("theme-family-grid");
  if (!grid) return;
  grid.innerHTML = THEME_FAMILIES.map(
      (f: { id: string; label: string }) => `<button type="button" class="select-card" data-family="${f.id}" aria-pressed="false">
        <span class="select-card-check">${CHECK_SVG}</span>
        <span class="family-preview" data-theme="${f.id}-light">
          <span class="fp-dots">
            <span class="fp-dot bg-primary"></span>
            <span class="fp-dot bg-secondary"></span>
            <span class="fp-dot bg-accent"></span>
            <span class="fp-dot bg-muted"></span>
          </span>
          <span class="fp-bar w-full"></span>
          <span class="fp-bar w-2/3"></span>
        </span>
        <span class="text-xs">${f.label}</span>
      </button>`,
    )
    .join("");
  renderThemeFamilySelection();
}

function renderThemeFamilySelection(): void {
  const current = getStoredFamily();
  document.querySelectorAll<HTMLElement>("#theme-family-grid [data-family]").forEach((el) => {
    const selected = el.dataset.family === current;
    el.classList.toggle("selected", selected);
    el.setAttribute("aria-pressed", String(selected));
  });
}

// ============ 语言卡片 ============
// 设置项三选一（system/en/zh-CN）持久化在 LauncherConfig.language；
// 切换编排（setLanguage）在 shell，这里只渲染选中态
export function renderLanguageCards(): void {
  for (const m of ["system", "en", "zh-CN"]) {
    document.getElementById(`lang-card-${m}`)?.classList.toggle("selected", m === state.languageSetting);
  }
}

// ============ 配置管理 ============
export function readConfigFromUI(): LauncherConfig {
  return {
    taskboard_path: (document.getElementById("cfg-path") as HTMLInputElement).value,
    node_path: (document.getElementById("cfg-node") as HTMLInputElement).value,
    codex_app_path: (document.getElementById("cfg-codex") as HTMLInputElement).value,
    taskboard_host: (document.getElementById("cfg-host") as HTMLInputElement).value || "127.0.0.1",
    taskboard_port: parseInt((document.getElementById("cfg-port") as HTMLInputElement).value) || 47823,
    cdp_port: parseInt((document.getElementById("cfg-cdp") as HTMLInputElement).value) || 9231,
    auto_open: (document.getElementById("toggle-auto-open") as HTMLInputElement).checked,
    separate_window_mode: (document.getElementById("toggle-mode") as HTMLInputElement).checked,
    minimize_to_tray_on_close: (document.getElementById("toggle-tray") as HTMLInputElement).checked,
    language: state.languageSetting,
    codex_guard: state.guardState,
  };
}

// 自启开关不落 LauncherConfig：OS 注册项是唯一事实来源（同 fastctx 接入状态的哲学）
// checkbox 已先翻转，失败时回退
export async function toggleAutostart(): Promise<void> {
  const el = document.getElementById("toggle-autostart") as HTMLInputElement;
  const next = el.checked;
  try {
    await invoke("autostart_set", { enabled: next });
  } catch (e) {
    el.checked = !next;
    toast(String(e), "error");
  }
}

export async function openLogDir(): Promise<void> {
  try {
    const dir = await invoke<string>("get_log_dir");
    await openUrl(dir);
  } catch (e) {
    toast(String(e), "error");
  }
}

export function onConfigChange(): void {
  validatePaths();
}

export async function saveConfig(): Promise<void> {
  const cfg = readConfigFromUI();
  try {
    // 使用 update_settings 而非 save_config：只更新设置类字段，
    // 保留 codex_guard 等看守状态不变，避免设置页保存回滚 apply/lock 状态
    await invoke("update_settings", { config: cfg });
    // 保存后同步后端最新的完整配置到前端 guardState，保持一致
    const latest = await invoke<LauncherConfig>("load_config");
    state.guardState = latest.codex_guard;
    toast(t("Settings saved"), "success");
  } catch (e) {
    toast(t("Save failed: {{error}}", { error: String(e) }), "error");
  }
}

// checkbox 由 <label> 包裹自动翻转，change 事件只负责联动标签与校验；
// 读取/写入统一走 .checked（不再操作 class）
export function updateModeLabel(): void {
  const toggle = document.getElementById("toggle-mode") as HTMLInputElement;
  const label = document.getElementById("toggle-mode-label")!;
  if (toggle.checked) {
    label.textContent = t("Separate window mode (does not restart Codex)");
  } else {
    label.textContent = t("Full launch mode (restarts Codex)");
  }
}

export function updateAutoOpenLabel(): void {
  const toggle = document.getElementById("toggle-auto-open") as HTMLInputElement;
  const label = document.getElementById("toggle-auto-open-label")!;
  if (toggle.checked) {
    label.textContent = t("Open browser automatically on start");
  } else {
    label.textContent = t("Do not open browser automatically");
  }
}

// ============ 路径验证 ============
export async function validatePaths(): Promise<void> {
  const cfg = readConfigFromUI();

  // 验证 taskboard 路径
  const pathEl = document.getElementById("validate-path")!;
  if (cfg.taskboard_path) {
    try {
      const valid = await invoke<boolean>("validate_taskboard_path", { path: cfg.taskboard_path });
      pathEl.textContent = valid ? t("Valid") : t("Invalid");
      pathEl.className = `config-validate ${valid ? "ok" : "err"}`;
    } catch {
      pathEl.textContent = t("Check failed");
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
    nodeEl.textContent = t("Unavailable");
    nodeEl.className = "config-validate err";
  }

  // 验证 codex app
  const codexEl = document.getElementById("validate-codex")!;
  if (cfg.codex_app_path) {
    try {
      const exists = await invoke<boolean>("check_codex_app", { appPath: cfg.codex_app_path });
      codexEl.textContent = exists ? t("Exists") : t("Not found");
      codexEl.className = `config-validate ${exists ? "ok" : "err"}`;
    } catch {
      codexEl.textContent = t("Check failed");
      codexEl.className = "config-validate err";
    }
  } else {
    codexEl.textContent = "";
    codexEl.className = "config-validate";
  }
}
