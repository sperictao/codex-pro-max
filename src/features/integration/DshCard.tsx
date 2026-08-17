// dsh 卡片：DeepSeek Harness 远程访问（Tailscale）——状态链、时间轴安装进度、开机自启
// 时间轴步骤由事件桥写入 store.dshTimeline；未跑过一键安装时用检测结果推导就绪视图（hasRunSetup 语义保留）

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { BTN, BTN_PRIMARY, BTN_SM, TOGGLE } from "@/shared/lib/ui";
import type { DshStatus, DshStepEvent } from "@/shared/types";

// 时间轴步骤顺序（与 Rust dsh_setup 的 index 一一对应）
const STEP_IDS = ["node", "install", "plugins", "tailscale", "magicdns", "start", "serve", "verify"] as const;

// 本地一键启动的时间轴步骤（与 Rust dsh_start_web 的 LOCAL_STEPS 一一对应）
const LOCAL_STEP_IDS = ["node", "install", "start", "ready"] as const;

// 步骤标题（key 即 i18n key）
const STEP_TITLES: Record<string, string> = {
  node: "Check Node.js & npm",
  install: "Install DeepSeek Harness (dsh)",
  plugins: "Install authorization plugins",
  tailscale: "Check Tailscale",
  magicdns: "Enable MagicDNS",
  start: "Start dsh Web",
  serve: "Configure Tailscale serve",
  verify: "Verify remote access",
  ready: "Local access ready",
};

export function statusTextKey(s: DshStatus): string {
  if (!s.nodeAvailable) return "Node.js not detected";
  if (!s.dshInstalled) return "DeepSeek Harness not installed";
  if (!s.dshCompatible) return "dsh version is not supported by the auth plugins";
  if (!s.tailscaleInstalled || !s.tailscaleOnline) return "Tailscale not ready";
  if (!s.magicDnsEnabled) return "MagicDNS not enabled";
  if (!s.dshRunning) return "dsh web not running";
  // 授权插件只服务于远程访问链路；纯本地用 dsh 不需要，故放在运行之后
  if (!s.pluginsInstalled) return "dsh auth plugins not installed";
  if (!s.serveConfigured) return "Tailscale serve not configured";
  return "Remote access ready";
}

// 由检测结果推导「就绪时间轴」：已满足的步骤标 done，其余 pending
export function timelineFromStatus(s: DshStatus): DshStepEvent[] {
  const allReady =
    s.nodeAvailable && s.dshInstalled && s.dshCompatible && s.pluginsInstalled &&
    s.dshRunning && s.tailscaleOnline && s.magicDnsEnabled && s.serveConfigured;
  const done = (ok: boolean): DshStepEvent["state"] => (ok ? "done" : "pending");
  const step = (index: number, id: string, ok: boolean): DshStepEvent => ({
    index, id, state: done(ok), detail: null, problem: null, solution: null,
  });
  return [
    step(0, "node", s.nodeAvailable),
    step(1, "install", s.dshInstalled && s.dshCompatible),
    step(2, "plugins", s.pluginsInstalled),
    step(3, "tailscale", s.tailscaleInstalled && s.tailscaleOnline),
    step(4, "magicdns", s.magicDnsEnabled),
    step(5, "start", s.dshRunning),
    step(6, "serve", s.serveConfigured),
    step(7, "verify", allReady),
  ];
}

// 本地模式就绪时间轴：node / install / start / ready 四项，与远程 8 步不同
export function localTimelineFromStatus(s: DshStatus): DshStepEvent[] {
  const done = (ok: boolean): DshStepEvent["state"] => (ok ? "done" : "pending");
  const step = (index: number, id: string, ok: boolean): DshStepEvent => ({
    index, id, state: done(ok), detail: null, problem: null, solution: null,
  });
  return [
    step(0, "node", s.nodeAvailable),
    step(1, "install", s.dshInstalled && s.dshCompatible),
    step(2, "start", s.dshRunning),
    step(3, "ready", s.dshRunning),
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

// 单个可用地址行：地址 + 复制/打开（本地与远程各一行，互不混淆）
function AddressRow({
  url,
  onCopy,
  onOpen,
}: {
  url: string;
  onCopy: (u: string) => void;
  onOpen: (u: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-wrap items-center justify-end gap-1.5">
      <span className="shrink-0 rounded-full bg-primary/15 px-2.5 py-0.5 font-mono text-xs text-primary">{url}</span>
      <button className={BTN_SM} onClick={() => void onCopy(url)}>{t("Copy")}</button>
      <button className={BTN_SM} onClick={() => void onOpen(url)}>{t("Open")}</button>
    </div>
  );
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
  // 当前时间轴模式：local（本地一键启动 4 步）或 remote（远程一键配置 8 步）
  const [timelineMode, setTimelineMode] = useState<"local" | "remote">("remote");

  const refresh = useCallback(async () => {
    try {
      const s = await cmd.dshDetect();
      setStatus(s);
      if (!hasRunSetup) {
        setDshTimeline(
          timelineMode === "local" ? localTimelineFromStatus(s) : timelineFromStatus(s),
        );
      }
    } catch (e) {
      toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
    }
  }, [hasRunSetup, setDshTimeline, t, toast, timelineMode]);

  useEffect(() => {
    void refresh();
    // 仅挂载时检测一次；后续刷新由操作完成时触发（与旧行为一致）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const start = async () => {
    if (busy) return;
    setBusy(true);
    setHasRunSetup(true);
    setTimelineMode("remote");
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

  // 一键启动 dsh（纯本地）：后端幂等保证 3899 就绪并返回本地地址，这里只管打开浏览器；
  // 已在跑时等同于「打开 dsh Web」。结束后刷新状态，时间轴经 localTimelineFromStatus
  // 反映本地就绪（4 步，不含远程的 tailscale/plugins/serve 等）
  const startLocal = async () => {
    if (busy) return;
    setBusy(true);
    setHasRunSetup(true);
    setTimelineMode("local");
    // 初始化为全 pending，随后由后端 dsh-step 事件按 LOCAL_STEPS 推进
    setDshTimeline(LOCAL_STEP_IDS.map((id, index) => ({
      index, id, state: "pending" as const, detail: null, problem: null, solution: null,
    })));
    let succeeded = false;
    try {
      const url = await cmd.dshStartWeb();
      await openUrl(url);
      succeeded = true;
    } catch (e) {
      toast(t("dsh start failed: {{error}}", { error: String(e) }), "error");
    } finally {
      setBusy(false);
      // 成功后回到本地就绪视图；失败时保留事件时间轴（问题+解决方案持续可见）
      if (succeeded) setHasRunSetup(false);
      try {
        const s = await cmd.dshDetect();
        setStatus(s);
        if (succeeded) setDshTimeline(localTimelineFromStatus(s));
      } catch (e) {
        toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
      }
    }
  };

  const open = async (url: string) => {
    try {
      await openUrl(url);
    } catch (e) {
      toast(t("Failed to open: {{error}}", { error: String(e) }), "error");
    }
  };

  // 复制地址：Open 只会用系统默认浏览器打开，用户想把地址发到手机/
  // 换已配好代理规则的浏览器时需要手动复制
  const copyUrl = async (url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      toast(t("Address copied"), "info");
    } catch (e) {
      toast(t("Failed to copy: {{error}}", { error: String(e) }), "error");
    }
  };

  const stopLocal = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await cmd.dshStop();
      toast(t("dsh web stopped"), "info");
    } catch (e) {
      toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
    } finally {
      setBusy(false);
      // 停止后回到状态驱动时间轴，避免事件时间轴残留「已就绪」的历史状态
      setHasRunSetup(false);
      setTimelineMode("remote");
      try {
        const s = await cmd.dshDetect();
        setStatus(s);
        setDshTimeline(timelineFromStatus(s));
      } catch (e) {
        toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
      }
    }
  };

  // 远程访问开关：打开走一键安装全链路（dsh web + Tailscale Serve），
  // 关闭全停。开关状态由检测的 url（stack_ready && serveConfigured）驱动
  const toggleRemote = async () => {
    if (busy) return;
    if (status?.url != null) {
      setBusy(true);
      try {
        await cmd.dshStop();
        toast(t("dsh remote access disabled"), "info");
      } catch (e) {
        toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
      } finally {
        setBusy(false);
        setHasRunSetup(false);
        setTimelineMode("remote");
        try {
          const s = await cmd.dshDetect();
          setStatus(s);
          setDshTimeline(timelineFromStatus(s));
        } catch (e) {
          toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
        }
      }
    } else {
      await start();
    }
  };

  const repair = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const version = await cmd.dshUpdate();
      toast(t("dsh integration repaired for {{version}}", { version }), "success");
    } catch (e) {
      toast(t("dsh integration repair failed: {{error}}", { error: String(e) }), "error");
    } finally {
      setBusy(false);
      // 更新流程不走 dsh-step 事件流：回到状态驱动时间轴
      setHasRunSetup(false);
      setTimelineMode("remote");
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
          {status && (!status.dshCompatible || !status.pluginsInstalled) && (
            <button
              className="inline-flex shrink-0 cursor-pointer items-center gap-1 rounded-full bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground whitespace-nowrap transition-colors outline-none hover:bg-primary/90 focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
              disabled={busy}
              onClick={() => void repair()}
            >
              {t("Repair dsh stack ({{version}})", { version: status.supportedVersion })}
            </button>
          )}
        </div>
      </div>

      <div className="min-w-0">
        <div className="truncate text-sm">{statusText}</div>
        {status?.error && !busy && (
          <div className="mt-1 text-xs text-destructive">
            {t("dsh integration check failed: {{error}}", { error: status.error })}
          </div>
        )}
        <div className="mt-1 text-xs opacity-60">
          {t("Remote access to the dsh Web UI over Tailscale HTTPS: https://<hostname>.ts.net → dsh web :3899. Tailscale identity is authorized by bundled dsh plugins; remote privileged APIs stay denied.")}
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
        {status?.dshRunning ? (
          <button className={BTN} disabled={busy} onClick={() => void stopLocal()}>
            {t("One-click stop dsh web")}
          </button>
        ) : (
          <button className={BTN_PRIMARY} disabled={busy} onClick={() => void startLocal()}>
            {t("One-click start dsh web")}
          </button>
        )}
        <div className="ml-auto flex min-w-0 flex-col items-end gap-1.5">
          {status?.url && !busy && (
            <AddressRow url={status.url} onCopy={copyUrl} onOpen={open} />
          )}
        </div>
      </div>

      <label
        className="flex flex-1 cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3"
        id="dsh-remote-access-row"
      >
        <span className="flex flex-col gap-0.5">
          <span className="text-sm">{t("Remote access")}</span>
          <span className="text-sm">{t("One-click remote access")}</span>
          <span className="text-xs opacity-60">
            {t("Enable or disable remote access to the dsh Web UI over Tailscale HTTPS in one click.")}
          </span>
        </span>
        <input
          type="checkbox"
          className={TOGGLE}
          id="toggle-dsh-remote-access"
          checked={status?.url != null}
          disabled={busy}
          onChange={() => void toggleRemote()}
        />
      </label>

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
            <span className="text-sm">{t("Auto-start the authorized dsh web service in the background at login")}</span>
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
