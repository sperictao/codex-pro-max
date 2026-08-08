// updater：更新源健康检查、更新检查/下载/安装、关于页链接

import { invoke } from "@tauri-apps/api/core";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { t } from "./i18n";
import { toast } from "./core";

interface UpdaterConfigHealth {
  configured: boolean;
  message: string;
}

interface UpdaterHelpPaths {
  docsPath: string;
  templatePath: string;
}

export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string | null;
  hasUpdate: boolean;
  releaseNotes: string | null;
  message: string | null;
}

export interface DownloadProgress {
  stage: string;
  version: string;
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number | null;
  attempt: number;
  maxAttempts: number;
}

export async function checkUpdaterHealth(): Promise<void> {
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

export async function openUpdaterHelp(target: "docs" | "template"): Promise<void> {
  try {
    const paths = await invoke<UpdaterHelpPaths>("get_updater_help_paths");
    await openUrl(target === "docs" ? paths.docsPath : paths.templatePath);
  } catch (e) {
    toast(t("Failed to open help: {{error}}", { error: String(e) }), "error");
  }
}

// shell 只读（语言切换重渲染）；赋值只发生在本模块
export let pendingUpdateInfo: UpdateInfo | null = null;
let updateBusy = false;

export function renderUpdateInfo(info: UpdateInfo): void {
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

export function renderDownloadProgress(p: DownloadProgress): void {
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

export async function checkUpdate(silent = false): Promise<void> {
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

export async function onUpdateButton(): Promise<void> {
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
export async function openGithub(): Promise<void> {
  try {
    await openUrl("https://github.com");
  } catch (e) {
    toast(t("Failed to open link: {{error}}", { error: String(e) }), "error");
  }
}
