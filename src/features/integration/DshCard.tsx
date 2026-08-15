// dsh 卡片：DeepSeek Harness 远程访问（Tailscale）——状态链、时间轴安装进度、开机自启
// 时间轴步骤由事件桥写入 store.dshTimeline；未跑过一键安装时用检测结果推导就绪视图（hasRunSetup 语义保留）

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { BTN, BTN_PRIMARY, TOGGLE } from "@/shared/lib/ui";
import type { DshStatus, DshStepEvent } from "@/shared/types";

// 时间轴步骤顺序（与 Rust dsh_setup 的 index 一一对应）
const STEP_IDS = ["node", "install", "start", "tailscale", "magicdns", "proxy", "serve", "verify"] as const;

// 步骤标题（key 即 i18n key）
const STEP_TITLES: Record<string, string> = {
  node: "Check Node.js & npm",
  install: "Install DeepSeek Harness (dsh)",
  start: "Start dsh Web",
  tailscale: "Check Tailscale",
  magicdns: "Enable MagicDNS",
  proxy: "Start loopback proxy",
  serve: "Configure Tailscale serve",
  verify: "Verify remote access",
};

function statusTextKey(s: DshStatus): string {
  if (!s.nodeAvailable) return "Node.js not detected";
  if (!s.dshInstalled) return "DeepSeek Harness not installed";
  if (!s.dshRunning) return "dsh web not running";
  if (!s.tailscaleInstalled || !s.tailscaleOnline) return "Tailscale not ready";
  if (!s.magicDnsEnabled) return "MagicDNS not enabled";
  if (!s.proxyRunning) return "Loopback proxy not running";
  if (!s.serveConfigured) return "Tailscale serve not configured";
  return "Remote access ready";
}

// 由检测结果推导「就绪时间轴」：已满足的步骤标 done，其余 pending
function timelineFromStatus(s: DshStatus): DshStepEvent[] {
  const allReady =
    s.nodeAvailable && s.dshInstalled && s.dshRunning && s.tailscaleOnline &&
    s.magicDnsEnabled && s.proxyRunning && s.serveConfigured;
  const done = (ok: boolean): DshStepEvent["state"] => (ok ? "done" : "pending");
  const step = (index: number, id: string, ok: boolean): DshStepEvent => ({
    index, id, state: done(ok), detail: null, problem: null, solution: null,
  });
  return [
    step(0, "node", s.nodeAvailable),
    step(1, "install", s.dshInstalled),
    step(2, "start", s.dshRunning),
    step(3, "tailscale", s.tailscaleInstalled && s.tailscaleOnline),
    step(4, "magicdns", s.magicDnsEnabled),
    step(5, "proxy", s.proxyRunning),
    step(6, "serve", s.serveConfigured),
    step(7, "verify", allReady),
  ];
}

function StepMarker({ state }: { state: DshStepEvent["state"] }) {
  switch (state) {
    case "done":
      return <>✓</>;
    case "failed":
      return <>✕</>;
    case "running":
      return <span className="timeline-spinner"></span>;
    case "skipped":
      return <>–</>;
    default:
      return <>○</>;
  }
}

export function DshCard() {
  const { t } = useTranslation();
  const toast = useAppStore((s) => s.toast);
  const timeline = useAppStore((s) => s.dshTimeline);
  const setDshTimeline = useAppStore((s) => s.setDshTimeline);
  const [status, setStatus] = useState<DshStatus | null>(null);
  const [busy, setBusy] = useState(false);
  // 是否跑过一键安装流程：跑过则时间轴以事件流为准，否则用检测结果渲染就绪视图
  const [hasRunSetup, setHasRunSetup] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const s = await cmd.dshDetect();
      setStatus(s);
      if (!hasRunSetup) setDshTimeline(timelineFromStatus(s));
    } catch (e) {
      toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
    }
  }, [hasRunSetup, setDshTimeline, t, toast]);

  useEffect(() => {
    void refresh();
    // 仅挂载时检测一次；后续刷新由操作完成时触发（与旧行为一致）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const start = async () => {
    if (busy) return;
    setBusy(true);
    setHasRunSetup(true);
    // 初始化为全 pending，随后由后端 dsh-step 事件逐步推进
    setDshTimeline(STEP_IDS.map((id, index) => ({
      index, id, state: "pending" as const, detail: null, problem: null, solution: null,
    })));
    let succeeded = false;
    try {
      await cmd.dshSetup();
      toast(t("Remote access is ready"), "success");
      succeeded = true;
    } catch {
      /* 失败详情已由 dsh-step 事件渲染在时间轴节点上 */
    } finally {
      setBusy(false);
      // 成功后回到状态驱动视图；失败时保留事件时间轴（问题+解决方案持续可见）
      if (succeeded) setHasRunSetup(false);
      try {
        const s = await cmd.dshDetect();
        setStatus(s);
        if (succeeded) setDshTimeline(timelineFromStatus(s));
      } catch (e) {
        toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
      }
    }
  };

  const stop = async () => {
    try {
      await cmd.dshStop();
      toast(t("dsh remote access services stopped"), "info");
    } catch (e) {
      toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
    }
    // 停止后回到状态驱动时间轴，避免事件时间轴残留「已就绪」的历史状态
    setHasRunSetup(false);
    try {
      const s = await cmd.dshDetect();
      setStatus(s);
      setDshTimeline(timelineFromStatus(s));
    } catch (e) {
      toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
    }
  };

  const open = async () => {
    if (!status?.url) {
      toast(t("Remote URL not available yet; run the one-click setup first"), "error");
      return;
    }
    try {
      await openUrl(status.url);
    } catch (e) {
      toast(t("Failed to open: {{error}}", { error: String(e) }), "error");
    }
  };

  // 复制远程地址：Open 只会用系统默认浏览器打开，用户想把地址发到手机/
  // 换已配好代理规则的浏览器时需要手动复制
  const copyUrl = async () => {
    if (!status?.url) return;
    try {
      await navigator.clipboard.writeText(status.url);
      toast(t("Remote URL copied"), "info");
    } catch (e) {
      toast(t("Failed to copy: {{error}}", { error: String(e) }), "error");
    }
  };

  const update = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const version = await cmd.dshUpdate();
      toast(t("dsh updated to {{version}}", { version }), "success");
    } catch (e) {
      toast(t("dsh update failed: {{error}}", { error: String(e) }), "error");
    } finally {
      setBusy(false);
      // 更新流程不走 dsh-step 事件流：回到状态驱动时间轴
      setHasRunSetup(false);
      try {
        const s = await cmd.dshDetect();
        setStatus(s);
        setDshTimeline(timelineFromStatus(s));
      } catch (e) {
        toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
      }
    }
  };

  const toggleAutostart = async () => {
    if (!status) return;
    const next = !status.autostartEnabled;
    try {
      await cmd.dshSetAutostart(next);
      setStatus({ ...status, autostartEnabled: next });
      toast(next ? t("Auto-start enabled") : t("Auto-start disabled"), "success");
    } catch (e) {
      toast(t("Failed to change auto-start: {{error}}", { error: String(e) }), "error");
    }
  };

  const statusText = busy ? t("Working…") : status ? t(statusTextKey(status)) : t("Detecting…");

  return (
    <div className="rounded-xl border border-border bg-card text-card-foreground flex flex-col gap-3 p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="text-sm font-medium">{t("DeepSeek Harness")}</div>
        <div className="flex min-w-0 items-center justify-end gap-2">
          {status?.dshVersion && (
            <span className="shrink-0 rounded-full border border-border px-2.5 py-0.5 font-mono text-xs opacity-70">
              {status.dshVersion}
            </span>
          )}
          {status?.latestVersion && (
            <button
              className="inline-flex shrink-0 cursor-pointer items-center gap-1 rounded-full bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground whitespace-nowrap transition-colors outline-none hover:bg-primary/90 focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
              disabled={busy}
              onClick={() => void update()}
            >
              {t("Update to {{version}}", { version: status.latestVersion })}
            </button>
          )}
        </div>
      </div>

      <div className="min-w-0">
        <div className="truncate text-sm">{statusText}</div>
        {status?.error && !busy && (
          <div className="mt-1 text-xs text-destructive">
            {t("Version check failed: {{error}}", { error: status.error })}
          </div>
        )}
        <div className="mt-1 text-xs opacity-60">
          {t("Remote access to the dsh Web UI over Tailscale HTTPS: https://<hostname>.ts.net → loopback proxy :3898 → dsh web :3899. Timeline nodes show the problem and its solution if a step fails.")}
        </div>
        {status?.url && !busy && (
          <div className="mt-1 text-xs opacity-60">
            {t("URL won't open? Proxy tools (Shadowrocket / Clash / Surge) often hijack *.ts.net traffic — add a DIRECT rule for it on the client device.")}{" "}
            <a
              className="underline underline-offset-2 hover:opacity-80"
              href="https://github.com/sperictao/codex-pro-max/blob/main/docs/dsh-remote-access.md"
              onClick={(e) => {
                e.preventDefault();
                void openUrl("https://github.com/sperictao/codex-pro-max/blob/main/docs/dsh-remote-access.md");
              }}
            >
              {t("Troubleshooting guide")}
            </a>
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <button className={BTN_PRIMARY} disabled={busy} onClick={() => void start()}>
          {busy ? t("Working…") : t("One-click remote access")}
        </button>
        <button className={BTN} disabled={busy || !status || (!status.dshRunning && !status.proxyRunning)} onClick={() => void stop()}>
          {t("Stop")}
        </button>
        <div className="ml-auto flex items-center gap-2">
          {status?.url && !busy && (
            <span className="shrink-0 rounded-full bg-primary/15 px-2.5 py-0.5 font-mono text-xs text-primary">
              {status.url}
            </span>
          )}
          <button className={BTN} disabled={busy || !status?.url} onClick={() => void copyUrl()}>
            {t("Copy URL")}
          </button>
          <button className={BTN} disabled={busy || !status?.url} onClick={() => void open()}>
            {t("Open")}
          </button>
        </div>
      </div>

      <div className="border-t border-border pt-3">
        <div className="mb-2 text-sm font-medium">{t("Setup Progress")}</div>
        <div className="flex flex-col">
          {timeline.map((step) => (
            <div className="timeline-node" data-state={step.state} key={step.index}>
              <div className="timeline-marker">
                <StepMarker state={step.state} />
              </div>
              <div className="timeline-content">
                <div className="timeline-title">{t(STEP_TITLES[step.id] ?? step.id)}</div>
                {step.detail && <div className="timeline-detail">{step.detail}</div>}
                {step.state === "failed" && (step.problem || step.solution) && (
                  <div className="timeline-issue">
                    {step.problem && <div className="timeline-problem">{step.problem}</div>}
                    {step.solution && <div className="timeline-solution">{step.solution}</div>}
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>

      <div className="border-t border-border pt-3">
        <div className="mb-2 text-sm font-medium">{t("Boot Auto-start")}</div>
        <label className="flex flex-1 cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3" id="dsh-autostart-row">
          <span className="flex flex-col gap-0.5">
            <span className="text-sm">{t("Auto-start dsh web and the loopback proxy in the background at login")}</span>
            <span className="text-xs opacity-60">
              {t("Keeps remote access available without opening this app. Tailscale serve is managed by the Tailscale app itself.")}
            </span>
          </span>
          <input type="checkbox" className={TOGGLE} id="toggle-dsh-autostart"
            checked={status?.autostartEnabled ?? false} onChange={() => void toggleAutostart()} />
        </label>
      </div>
    </div>
  );
}
