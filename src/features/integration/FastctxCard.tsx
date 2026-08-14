// FastCtx 卡片：接入/摘除委托 fastctx CLI（ADR 0003）；状态以 config.toml 为准实时检测，不持久化开关
// 摘除走原生 ask 确认（保行为）；busy 期间开关受控不变（旧「回弹」语义）

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { BTN, TOGGLE } from "@/shared/lib/ui";
import type { FastctxStatus } from "@/shared/types";

export function FastctxCard() {
  const { t } = useTranslation();
  const toast = useAppStore((s) => s.toast);
  const [status, setStatus] = useState<FastctxStatus>({
    installed: false,
    version: null,
    integrated: false,
    latestVersion: null,
  });
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await cmd.fastctxDetect());
    } catch (e) {
      toast(t("fastctx detection failed: {{error}}", { error: String(e) }), "error");
    }
  }, [t, toast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const toggle = async () => {
    if (busy) return;
    if (status.integrated) {
      const ok = await ask(
        t("Unapply will stop fastctx processes and delete ~/.fastctx managed data (the npm package stays and can be re-integrated anytime). Codex configuration written by fastctx will be removed.\n\nProceed with unapply?"),
        { title: t("Unapply fastctx"), kind: "warning" },
      );
      if (!ok) return;
    }
    setBusy(true);
    try {
      let st = status;
      if (!st.installed) {
        await cmd.fastctxInstall();
        toast(t("fastctx installed; integrating…"), "info");
        st = await cmd.fastctxDetect();
        setStatus(st);
      }
      if (st.integrated) {
        await cmd.fastctxUnapply();
        toast(t("fastctx unapplied; restart Codex sessions to take full effect"), "info");
      } else {
        const res = await cmd.fastctxApply();
        toast(t("fastctx integrated; restart Codex sessions to activate"), "success");
        if (!res.selfCheckPassed) {
          const line =
            res.selfCheckOutput.split("\n").find((l) => l.includes("[FAIL]")) ??
            res.selfCheckOutput.split("\n")[0] ??
            "";
          toast(t("fastctx self-check failed: {{line}} (open the console to troubleshoot)", { line }), "error");
        }
      }
    } catch (e) {
      toast(t("fastctx operation failed: {{error}}", { error: String(e) }), "error");
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const openConsole = async () => {
    if (!status.installed) {
      toast(t("fastctx not detected; turn on the integration toggle to install it automatically"), "error");
      return;
    }
    try {
      await cmd.fastctxOpenConsole();
    } catch (e) {
      toast(t("Failed to open console: {{error}}", { error: String(e) }), "error");
    }
  };

  const statusText = busy
    ? t("Working…")
    : !status.installed
      ? t("Not installed")
      : status.integrated
        ? `${t("Integrated")}${status.version ? ` · ${status.version}` : ""}`
        : t("Installed{{version}}, not integrated", { version: status.version ? ` (${status.version})` : "" });

  return (
    <div className="mt-4 rounded-xl border border-border bg-card text-card-foreground flex flex-col gap-3 p-4">
      <div className="text-sm font-medium">FastCtx</div>

      <label className="flex flex-1 cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3" id="fastctx-row">
        <span className="flex flex-col gap-0.5">
          <span className="text-sm">{t("Integrate fastctx repo tools (MCP)")}</span>
          <span className="text-xs opacity-60">
            {t("Provides structured read/grep/glob/replace/run tools for Codex. Integrate = fastctx apply; unapply = fastctx unapply (removes ~/.fastctx managed data; the npm package stays and can be re-integrated anytime).")}
          </span>
        </span>
        <input type="checkbox" className={TOGGLE} id="toggle-fastctx"
          checked={status.integrated} onChange={() => void toggle()} />
      </label>

      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm">{statusText}</div>
          {!status.installed && (
            <div className="mt-1.5 text-xs opacity-60">
              {t("fastctx not detected; turning on the toggle will install it automatically via")}{" "}
              <span className="font-mono">npm install --global fastctx</span>{" "}
              {t("(requires Node.js 18+).")}
            </div>
          )}
        </div>
        {status.latestVersion && (
          <span className="shrink-0 rounded-full bg-primary/15 px-2.5 py-0.5 font-mono text-xs text-primary">
            {`v${status.latestVersion}`}
          </span>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <button className={BTN} onClick={() => void openConsole()}>{t("Open fastctx Console")}</button>
        <span className="text-xs opacity-60">
          {t("Output tier, background jobs and updates are managed in the fastctx console")}
        </span>
      </div>
    </div>
  );
}
