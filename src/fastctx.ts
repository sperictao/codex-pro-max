// fastctx：接入/摘除委托 fastctx CLI（ADR 0003）；状态以 config.toml 为准实时检测，不持久化开关

import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import { toast } from "./core";

interface FastctxStatus {
  installed: boolean;
  version: string | null;
  integrated: boolean;
}

interface FastctxApplyResult {
  selfCheckPassed: boolean;
  selfCheckOutput: string;
}

let fastctxState: FastctxStatus = { installed: false, version: null, integrated: false };
let fastctxBusy = false;

export function renderFastctx(): void {
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

export async function refreshFastctxStatus(): Promise<void> {
  try {
    fastctxState = await invoke<FastctxStatus>("fastctx_detect");
  } catch (e) {
    toast(t("fastctx detection failed: {{error}}", { error: String(e) }), "error");
  }
  renderFastctx();
}

export async function toggleFastctx(): Promise<void> {
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

export async function openFastctxConsole(): Promise<void> {
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
