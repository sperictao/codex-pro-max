// FastCtx 卡片：接入/摘除委托 fastctx CLI（ADR 0003）；状态以 config.toml 为准实时检测，不持久化开关
// 摘除走原生 ask 确认（保行为）；busy 期间开关受控不变（旧「回弹」语义）

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import * as cmd from "@/shared/commands";
import { Badge } from "@/shared/components/ui/badge";
import { Button } from "@/shared/components/ui/button";
import { Card, CardTitle } from "@/shared/components/ui/card";
import { Switch } from "@/shared/components/ui/switch";
import type { FastctxStatus } from "@/shared/types";

export function FastctxCard() {
  const { t } = useTranslation();
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
      toast.error(t("fastctx detection failed: {{error}}", { error: String(e) }));
    }
  }, [t]);

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
        toast.info(t("fastctx installed; integrating…"));
        st = await cmd.fastctxDetect();
        setStatus(st);
      }
      if (st.integrated) {
        await cmd.fastctxUnapply();
        toast.info(t("fastctx unapplied; restart Codex sessions to take full effect"));
      } else {
        const res = await cmd.fastctxApply();
        toast.success(t("fastctx integrated; restart Codex sessions to activate"));
        if (!res.selfCheckPassed) {
          const line =
            res.selfCheckOutput.split("\n").find((l) => l.includes("[FAIL]")) ??
            res.selfCheckOutput.split("\n")[0] ??
            "";
          toast.error(t("fastctx self-check failed: {{line}} (open the console to troubleshoot)", { line }));
        }
      }
    } catch (e) {
      toast.error(t("fastctx operation failed: {{error}}", { error: String(e) }));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const openConsole = async () => {
    if (!status.installed) {
      toast.error(t("fastctx not detected; turn on the integration toggle to install it automatically"));
      return;
    }
    try {
      await cmd.fastctxOpenConsole();
    } catch (e) {
      toast.error(t("Failed to open console: {{error}}", { error: String(e) }));
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
    <Card className="px-(--card-spacing) shadow-xs">
      <CardTitle className="text-sm">FastCtx</CardTitle>

      <label htmlFor="toggle-fastctx" className="flex flex-1 cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3" id="fastctx-row">
        <span className="flex flex-col gap-0.5">
          <span className="text-sm">{t("Integrate fastctx repo tools (MCP)")}</span>
          <span className="text-xs text-muted-foreground">
            {t("Provides structured read/grep/glob/replace/run tools for Codex. Integrate = fastctx apply; unapply = fastctx unapply (removes ~/.fastctx managed data; the npm package stays and can be re-integrated anytime).")}
          </span>
        </span>
        <Switch id="toggle-fastctx" checked={status.integrated} onCheckedChange={() => void toggle()} />
      </label>

      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm tabular-nums">{statusText}</div>
          {!status.installed && (
            <div className="mt-1.5 text-xs tabular-nums text-muted-foreground">
              {t("fastctx not detected; turning on the toggle will install it automatically via")}{" "}
              <span className="font-mono">npm install --global fastctx</span>{" "}
              {t("(requires Node.js 18+).")}
            </div>
          )}
        </div>
        {status.latestVersion && (
          <Badge variant="secondary" className="shrink-0 font-mono tabular-nums">
            {`v${status.latestVersion}`}
          </Badge>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button variant="outline" onClick={() => void openConsole()}>{t("Open fastctx Console")}</Button>
        <span className="text-xs text-muted-foreground">
          {t("Output tier, background jobs and updates are managed in the fastctx console")}
        </span>
      </div>
    </Card>
  );
}
