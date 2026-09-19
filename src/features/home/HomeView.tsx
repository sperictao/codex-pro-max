// 主页：总状态指示器 + taskboard/injector 进程卡 + 全局启停 + Codex 重启确认弹窗
// 行为依据旧 service.ts（按钮禁用逻辑、状态聚合、CODEX_NO_CDP 重启流程逐条保留）

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useAppStore } from "@/shared/store";
import { currentConfigDraft } from "@/shared/config";
import * as cmd from "@/shared/commands";
import { Modal } from "@/shared/components/Modal";
import { Button } from "@/shared/components/ui/button";
import { Card, CardAction, CardContent, CardHeader, CardTitle } from "@/shared/components/ui/card";
import type { ProcessInfo, ProcessStatus } from "@/shared/types";

// 后端错误前缀：Codex 已运行但未开 CDP（仅 Windows 发出）
const CODEX_NO_CDP_MARK = "CODEX_RUNNING_NO_CDP|";

type ServiceKey = "taskboard" | "injector";

const STATUS_TEXT: Record<ProcessStatus, string> = {
  running: "Running",
  stopped: "Stopped",
  starting: "Starting",
  stopping: "Stopping",
  failed: "Failed",
};

const ICON_CHECK = (
  <svg width="56" height="56" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="m5 12 4 4L19 6" /></svg>
);
const ICON_X = (
  <svg width="56" height="56" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M6 18 18 6M6 6l12 12" /></svg>
);
const ICON_PLAY = (
  <svg width="56" height="56" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M8.5 5.5c-.8-.5-1.9 0-1.9 1v11c0 1 1.1 1.6 1.9 1.1l8.5-5.5c.8-.5.8-1.7 0-2.2L8.5 5.5z" /></svg>
);

type IndicatorState = "stopped" | "starting" | "running" | "failed";

const INDICATOR_ICONS: Record<IndicatorState, React.ReactNode> = {
  running: ICON_CHECK,
  stopped: ICON_X,
  starting: ICON_PLAY,
  failed: ICON_X,
};

// 总状态聚合（旧 updateServiceStatusIndicator 逻辑）
function aggregate(statuses: ProcessStatus[]): { state: IndicatorState; textKey: string } {
  const hasTransition = statuses.some((s) => s === "starting" || s === "stopping");
  const hasRunning = statuses.some((s) => s === "running");
  if (statuses.includes("failed")) return { state: "failed", textKey: "Service issue" };
  if (hasTransition) return { state: "starting", textKey: hasRunning ? "Partially running" : "Services starting" };
  if (statuses.every((s) => s === "running")) return { state: "running", textKey: "All services running" };
  if (hasRunning) return { state: "starting", textKey: "Partially running" };
  return { state: "stopped", textKey: "Services stopped" };
}

function ServiceCard({
  kind,
  info,
  received,
  onStart,
  onStop,
  onOpen,
}: {
  kind: ServiceKey;
  info: ProcessInfo;
  received: boolean;
  onStart: () => void;
  onStop: () => void;
  onOpen?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Card className="shadow-xs">
      <CardHeader>
        <CardTitle className="text-sm">{kind === "taskboard" ? t("Taskboard Server") : t("Codex Injector")}</CardTitle>
        <CardAction>
          <div className={`status-badge ${info.status}`}>
            <span className="dot"></span>
            <span>{t(STATUS_TEXT[info.status] ?? STATUS_TEXT.stopped)}</span>
          </div>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="min-h-8 truncate font-mono text-xs tabular-nums text-muted-foreground">
          {info.message || (received ? "-" : t("Not started"))}
        </div>
        <div className="flex gap-2">
          <Button variant="outline" disabled={info.status === "running" || info.status === "starting"} onClick={onStart}>
            {t("Start")}
          </Button>
          <Button variant="outline" disabled={info.status !== "running"} onClick={onStop}>
            {t("Stop")}
          </Button>
          {onOpen && (
            <Button variant="outline" disabled={info.status !== "running"} onClick={onOpen}>
              {t("Open")}
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

export function HomeView() {
  const { t } = useTranslation();
  const services = useAppStore((s) => s.services);
  const servicesReceived = useAppStore((s) => s.servicesReceived);
  const refreshStatus = useAppStore((s) => s.refreshStatus);
  const goto = useAppStore((s) => s.goto);

  const [startAllBusy, setStartAllBusy] = useState(false);
  const [stopAllBusy, setStopAllBusy] = useState(false);

  // Codex 重启确认（应用内弹窗，确认键聚焦；确认后 quit_codex 重试并显示加载遮罩）
  const [restartAskOpen, setRestartAskOpen] = useState(false);
  const [restartLoading, setRestartLoading] = useState(false);
  const restartResolver = useRef<((yes: boolean) => void) | null>(null);
  const restartConfirmRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (restartAskOpen) restartConfirmRef.current?.focus();
  }, [restartAskOpen]);

  const askRestart = () =>
    new Promise<boolean>((resolve) => {
      restartResolver.current = resolve;
      setRestartAskOpen(true);
    });
  const settleRestart = (yes: boolean) => {
    setRestartAskOpen(false);
    restartResolver.current?.(yes);
    restartResolver.current = null;
  };

  const startWithCodexRestart = async (fn: () => Promise<unknown>): Promise<boolean> => {
    try {
      await fn();
      return true;
    } catch (e) {
      if (!String(e).includes(CODEX_NO_CDP_MARK)) throw e;
      if (!(await askRestart())) return false;
      setRestartLoading(true);
      try {
        await cmd.quitCodex();
        await fn();
      } finally {
        setRestartLoading(false);
      }
      return true;
    }
  };

  const draft = () => currentConfigDraft(useAppStore.getState());

  const onStartAll = async () => {
    const cfg = draft();
    if (!cfg.taskboard_path) {
      toast.error(t("Please configure the Taskboard path in Settings first"));
      goto("settings", "general");
      return;
    }
    setStartAllBusy(true);
    try {
      await cmd.updateSettings(cfg);
      if (!(await startWithCodexRestart(() => cmd.startAll(cfg)))) {
        toast.info(t("Launch cancelled"));
        return;
      }
      toast.success(t("All services started"));
      await refreshStatus();
    } catch (e) {
      toast.error(t("Launch failed: {{error}}", { error: String(e) }));
    } finally {
      setStartAllBusy(false);
    }
  };

  const onStopAll = async () => {
    setStopAllBusy(true);
    try {
      await cmd.stopAll();
      toast.info(t("All services stopped"));
      await refreshStatus();
    } catch (e) {
      toast.error(t("Stop failed: {{error}}", { error: String(e) }));
    } finally {
      setStopAllBusy(false);
    }
  };

  const onStart = (kind: ServiceKey) => async () => {
    const cfg = draft();
    try {
      if (kind === "taskboard") {
        await cmd.startTaskboard(cfg);
        toast.success(t("Taskboard server started"));
      } else {
        if (!(await startWithCodexRestart(() => cmd.startInjector(cfg)))) {
          toast.info(t("Launch cancelled"));
          return;
        }
        toast.success(t("Codex injector started"));
      }
      await refreshStatus();
    } catch (e) {
      toast.error(t("Launch failed: {{error}}", { error: String(e) }));
    }
  };

  const onStop = (kind: ServiceKey) => async () => {
    try {
      if (kind === "taskboard") {
        await cmd.stopTaskboard();
        toast.info(t("Taskboard server stopped"));
      } else {
        await cmd.stopInjector();
        toast.info(t("Codex injector stopped"));
      }
      await refreshStatus();
    } catch (e) {
      toast.error(t("Stop failed: {{error}}", { error: String(e) }));
    }
  };

  const onOpenTaskboard = async () => {
    try {
      await cmd.openTaskboard(draft());
    } catch (e) {
      toast.error(t("Open failed: {{error}}", { error: String(e) }));
    }
  };

  const list = [services.taskboard, services.injector];
  const indicator = aggregate(list.map((s) => s.status));
  const anyRunning = list.some((s) => s.status === "running" || s.status === "starting");
  const allStopped = list.every((s) => s.status === "stopped" || s.status === "failed");

  return (
    <main className="app-page-scroll flex-1 overflow-y-auto" id="main-view">
      <div className="status-indicator" id="service-status-indicator" role="status" aria-live="polite">
        <div className="status-indicator-icon-container">
          <div className={`status-indicator-icon ${indicator.state}`} aria-hidden="true">
            <div className="status-indicator-symbol">{INDICATOR_ICONS[indicator.state]}</div>
          </div>
        </div>
        <div className={`status-indicator-text ${indicator.state}`}>{t(indicator.textKey)}</div>
      </div>

      <div className="mb-4 text-sm font-semibold">{t("Service Status")}</div>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <ServiceCard kind="taskboard" info={services.taskboard} received={servicesReceived.taskboard}
          onStart={() => void onStart("taskboard")()} onStop={() => void onStop("taskboard")()}
          onOpen={() => void onOpenTaskboard()} />
        <ServiceCard kind="injector" info={services.injector} received={servicesReceived.injector}
          onStart={() => void onStart("injector")()} onStop={() => void onStop("injector")()} />
      </div>

      <div className="mt-6 flex justify-center gap-4">
        <Button size="lg" className="h-12 w-64" id="btn-start-all" disabled={anyRunning || startAllBusy} onClick={() => void onStartAll()}>
          {startAllBusy ? t("Starting...") : t("Start All")}
        </Button>
        <Button variant="destructive" size="lg" className="h-12 w-64" id="btn-stop-all" disabled={allStopped || stopAllBusy} onClick={() => void onStopAll()}>
          {stopAllBusy ? t("Stopping...") : t("Stop All")}
        </Button>
      </div>

      <Modal open={restartAskOpen} labelledBy="codex-restart-modal-title" title={t("Restart Codex")} cardClassName="codex-modal-lg">
        <div className="flex items-start gap-4">
          <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-lg bg-muted text-primary" aria-hidden="true">
            <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z" /><path d="M12 9v4" /><path d="M12 17h.01" /></svg>
          </div>
          <div className="flex flex-col gap-1.5">
            <h3 className="text-base font-semibold" id="codex-restart-modal-title">{t("Restart Codex")}</h3>
            <p className="text-sm leading-relaxed text-muted-foreground">
              {t("Codex is running without the debug port, so the injector cannot connect. All current Codex windows will be closed. Continue?")}
            </p>
          </div>
        </div>
        <div className="mt-3 flex justify-end gap-2">
          <Button variant="outline" size="lg" className="px-4" onClick={() => settleRestart(false)}>
            {t("Cancel")}
          </Button>
          <Button
            ref={restartConfirmRef}
            size="lg"
            className="px-4"
            onClick={() => settleRestart(true)}
          >
            {t("Restart Codex")}
          </Button>
        </div>
      </Modal>

      {restartLoading && (
        <div className="modal-overlay loading-overlay">
          <div className="loading-card">
            <div className="loading-spinner" aria-hidden="true"></div>
            <div className="loading-text">{t("Restarting Codex...")}</div>
          </div>
        </div>
      )}
    </main>
  );
}
