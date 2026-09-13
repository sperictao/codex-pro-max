// Skill 视图：状态检测徽标 + 安装（旧 service.ts refreshSkillStatus/installSkill）
// 视图挂载即刷新（旧行为：点击导航进入时触发 refreshSkillStatus）

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useAppStore } from "@/shared/store";
import { currentConfigDraft } from "@/shared/config";
import * as cmd from "@/shared/commands";
import { Button } from "@/shared/components/ui/button";
import { Card, CardContent, CardHeader } from "@/shared/components/ui/card";
import type { SkillStatus } from "@/shared/types";

export function SkillView() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<SkillStatus | null>(null);
  const [detectError, setDetectError] = useState<string | null>(null);
  const [result, setResult] = useState("");

  const refresh = useCallback(async () => {
    try {
      const cfg = currentConfigDraft(useAppStore.getState());
      setStatus(await cmd.checkSkillStatus(cfg.taskboard_path));
      setDetectError(null);
    } catch (e) {
      setStatus(null);
      setDetectError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const install = async () => {
    const cfg = currentConfigDraft(useAppStore.getState());
    if (!cfg.taskboard_path) {
      toast.error(t("Please configure the Taskboard path first"));
      return;
    }
    try {
      setResult(await cmd.installSkill(cfg.taskboard_path));
      toast.success(t("Skill installed successfully"));
    } catch (e) {
      setResult(t("Failed: {{error}}", { error: String(e) }));
      toast.error(t("Installation failed: {{error}}", { error: String(e) }));
    }
    await refresh();
  };

  const badgeCls = detectError
    ? "failed"
    : status === null
      ? "stopped"
      : status.state === "installed"
        ? "running"
        : status.state === "mismatch"
          ? "starting"
          : "stopped";
  const badgeText = detectError
    ? t("Detection failed")
    : status === null
      ? t("Detecting…")
      : status.state === "installed"
        ? t("Installed")
        : status.state === "mismatch"
          ? t("Installation mismatch")
          : t("Not installed");
  const detail = detectError ?? status?.detail ?? "";

  return (
    <main className="flex-1 overflow-y-auto p-4 md:p-6" id="skill-view">
      <h2 className="mb-4 text-base font-semibold">Codex Skill</h2>
      <Card className="max-w-2xl shadow-xs">
        <CardHeader>
          <div className="flex items-center gap-3">
            <span className={`status-badge ${badgeCls}`} id="skill-status-badge">
              <span className="dot"></span>
              <span>{badgeText}</span>
            </span>
            <span className="font-mono text-xs text-muted-foreground">~/.codex/skills/manage-taskboard</span>
          </div>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <p className="text-sm tabular-nums text-muted-foreground">{detail}</p>
          <p className="text-sm leading-6 text-muted-foreground">
            {t("Install the")} <code className="rounded bg-muted px-1 font-mono text-xs">manage-taskboard</code>{" "}
            {t("Skill into Codex. Once installed, you can create, view and manage Taskboard tasks directly in Codex. Installation creates a symlink at")}{" "}
            <code className="rounded bg-muted px-1 font-mono text-xs">~/.codex/skills/manage-taskboard</code>{" "}
            {t("pointing to the Taskboard repository, so the Skill stays in sync with Taskboard updates.")}
          </p>
          <div className="flex flex-col gap-2">
            <div>
              <Button id="btn-install-skill" onClick={() => void install()}>
                {t("Install Skill")}
              </Button>
            </div>
            <div className="font-mono text-xs tabular-nums whitespace-pre-wrap text-muted-foreground">{result}</div>
          </div>
        </CardContent>
      </Card>
    </main>
  );
}
