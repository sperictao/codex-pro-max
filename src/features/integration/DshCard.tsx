// dsh 卡片：DeepSeek Harness 访问模式（本地/远程）切换 + 一键启动/关闭 + 状态链时间轴
// 模式开关只是选择访问模式（本地 = 127.0.0.1:3899，远程 = 追加 Tailscale HTTPS），
// 不执行任何启用/停止；一键启动/关闭按钮按当前模式走对应流程。
// 时间轴步骤由事件桥写入 store.dshTimeline；未跑过流程时用检测结果推导就绪视图

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { BTN_DESTRUCTIVE, BTN_PRIMARY, BTN_SM, TOGGLE } from "@/shared/lib/ui";
import type { DshStatus, DshStepEvent } from "@/shared/types";

export type DshAccessMode = "local" | "remote";

// 远程时间轴步骤顺序（与 Rust dsh_setup 的 index 一一对应）
const STEP_IDS = ["node", "install", "plugins", "tailscale", "magicdns", "start", "serve", "verify"] as const;

// 本地一键启动的时间轴步骤（与 Rust dsh_start_web 的 LOCAL_STEPS 一一对应）
const LOCAL_STEP_IDS = ["node", "install", "start", "ready"] as const;

// 步骤标题（key 即 i18n key；本地四步 node/install/start/ready 也在其中）
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

// 本地模式状态文案：不要求 Tailscale/插件/serve，只看 node、dsh 安装与 web 是否在跑
export function localStatusTextKey(s: DshStatus): string {
  if (!s.nodeAvailable) return "Node.js not detected";
  if (!s.dshInstalled) return "DeepSeek Harness not installed";
  if (!s.dshCompatible) return "dsh version is not supported by the auth plugins";
  if (!s.dshRunning) return "dsh web not running";
  return "Local access ready";
}

// 由检测结果推导「就绪时间轴」：已满足的步骤标 done，其余 pending（远程 8 步）
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
  // 启动/修复与停止各自维护 busy（同主页 Start All / Stop All 语义），互斥防并发
  const [startBusy, setStartBusy] = useState(false);
  const [stopBusy, setStopBusy] = useState(false);
  const busy = startBusy || stopBusy;
  // 是否跑过一键流程：跑过则时间轴以事件流为准，否则用检测结果渲染就绪视图
  const [hasRunSetup, setHasRunSetup] = useState(false);
  // 当前访问模式：local（127.0.0.1:3899 本地访问）或 remote（Tailscale HTTPS 远程访问）。
  // 开关只切换模式，不执行任何启用/停止
  const [mode, setMode] = useState<DshAccessMode>("remote");

  const isRemote = mode === "remote";

  const refresh = useCallback(async () => {
    try {
      const s = await cmd.dshDetect();
      setStatus(s);
      if (!hasRunSetup) {
        setDshTimeline(isRemote ? timelineFromStatus(s) : localTimelineFromStatus(s));
      }
    } catch (e) {
      toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
    }
  }, [hasRunSetup, isRemote, setDshTimeline, t, toast]);

  useEffect(() => {
    void refresh();
    // 仅挂载时检测一次；后续刷新由操作完成时触发（与旧行为一致）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 切换访问模式：仅选择，不执行任何启动/停止；时间轴切到对应模式的就绪视图
  const switchMode = (next: DshAccessMode) => {
    if (next === mode || busy) return;
    setMode(next);
    setHasRunSetup(false);
    if (status) {
      setDshTimeline(next === "remote" ? timelineFromStatus(status) : localTimelineFromStatus(status));
    }
  };

  // 一键启动：按当前模式走对应启用流程。
  // 远程 → dsh_setup 全链路（dsh web + Tailscale Serve + 校验）；
  // 本地 → dsh_start_web（幂等保证 3899 就绪并返回本地地址，这里只管打开浏览器）。
  // 结束后刷新状态并按模式切回状态驱动时间轴
  const startCurrent = async () => {
    if (busy) return;
    setStartBusy(true);
    setHasRunSetup(true);
    const ids = isRemote ? STEP_IDS : LOCAL_STEP_IDS;
    // 初始化为全 pending，随后由后端 dsh-step 事件逐步推进
    setDshTimeline(ids.map((id, index) => ({
      index, id, state: "pending" as const, detail: null, problem: null, solution: null,
    })));
    let succeeded = false;
    try {
      if (isRemote) {
        await cmd.dshSetup();
        toast(t("Remote access ready"), "success");
      } else {
        const url = await cmd.dshStartWeb();
        await openUrl(url);
      }
      succeeded = true;
    } catch (e) {
      // 远程失败详情已由 dsh-step 事件渲染在时间轴节点上
      if (!isRemote) toast(t("dsh start failed: {{error}}", { error: String(e) }), "error");
    } finally {
      setStartBusy(false);
      // 成功后回到状态驱动视图；失败时保留事件时间轴（问题+解决方案持续可见）
      if (succeeded) setHasRunSetup(false);
      try {
        const s = await cmd.dshDetect();
        setStatus(s);
        if (succeeded) {
          setDshTimeline(isRemote ? timelineFromStatus(s) : localTimelineFromStatus(s));
        }
      } catch (e) {
        toast(t("dsh detection failed: {{error}}", { error: String(e) }), "error");
      }
    }
  };

  // 一键关闭：按当前模式关闭。Rust dsh_stop 对两种模式都是幂等的：
  // 本地模式会关掉 dsh web（serve/自启若从未配置则为 no-op）
  const stopCurrent = async () => {
    if (busy) return;
    setStopBusy(true);
    try {
      await cmd.dshStop();
      toast(t("dsh web stopped"), "info");
    } catch (e) {
      toast(t("Stop failed: {{error}}", { error: String(e) }), "error");
    } finally {
      setStopBusy(false);
      // 停止后回到状态驱动时间轴，避免事件时间轴残留「已就绪」的历史状态
      setHasRunSetup(false);
      try {
        const s = await cmd.dshDetect();
        setStatus(s);
        setDshTimeline(isRemote ? timelineFromStatus(s) : localTimelineFromStatus(s));
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

  const repair = async () => {
    if (busy) return;
    setStartBusy(true);
    try {
      const version = await cmd.dshUpdate();
      toast(t("dsh integration repaired for {{version}}", { version }), "success");
    } catch (e) {
      toast(t("dsh integration repair failed: {{error}}", { error: String(e) }), "error");
    } finally {
      setStartBusy(false);
      // 更新流程不走 dsh-step 事件流：回到状态驱动时间轴
      setHasRunSetup(false);
      try {
        const s = await cmd.dshDetect();
        setStatus(s);
        setDshTimeline(isRemote ? timelineFromStatus(s) : localTimelineFromStatus(s));
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

  // 当前模式的访问地址：本地模式在 dsh web 运行时显示 loopback 地址，
  // 远程模式在 serve 就绪且有 URL 时显示 tailnet HTTPS 地址
  const activeUrl = !busy
    ? isRemote
      ? status?.url ?? null
      : status?.dshRunning
        ? status?.localUrl ?? "http://127.0.0.1:3899"
        : null
    : null;

  const statusText = busy ? t("Working…") : status ? t(isRemote ? statusTextKey(status) : localStatusTextKey(status)) : t("Detecting…");

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
        {isRemote ? (
          <>
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
          </>
        ) : (
          <div className="mt-1 text-xs opacity-60">
            {t("Local access to the dsh Web UI at http://127.0.0.1:3899.")}
          </div>
        )}
      </div>

      <label
        className="flex flex-1 cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3"
        id="dsh-remote-access-row"
      >
        <span className="flex flex-col gap-0.5">
          <span className="text-sm">{t("Remote access")}</span>
          <span className="text-sm">
            {t(isRemote ? "Remote access mode" : "Local access mode")}
          </span>
          <span className="text-xs opacity-60">
            {t("Switching the access mode only selects the setup/close flow; click Start or Stop below to apply it. It does not start or stop anything by itself.")}
          </span>
        </span>
        <input
          type="checkbox"
          className={TOGGLE}
          id="toggle-dsh-remote-access"
          checked={isRemote}
          disabled={busy}
          onChange={(e) => switchMode(e.target.checked ? "remote" : "local")}
        />
      </label>

      <div className="flex flex-wrap items-center gap-2">
        <button
          className={BTN_PRIMARY}
          disabled={busy || !!status?.dshRunning}
          onClick={() => void startCurrent()}
        >
          {startBusy ? t("Starting...") : t("One-click start dsh web")}
        </button>
        <button
          className={BTN_DESTRUCTIVE}
          disabled={busy || !status?.dshRunning}
          onClick={() => void stopCurrent()}
        >
          {stopBusy ? t("Stopping...") : t("One-click stop dsh web")}
        </button>
        <div className="ml-auto flex min-w-0 flex-col items-end gap-1.5">
          {activeUrl && (
            <AddressRow url={activeUrl} onCopy={copyUrl} onOpen={open} />
          )}
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