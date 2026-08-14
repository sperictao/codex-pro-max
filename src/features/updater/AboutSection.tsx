// 关于分区（updater 域）：版本 + 更新状态聚合卡（源健康/版本对比/上次检查时间/失败原因持久展示）+ GitHub 链接
// 进度行可见性 = store.downloadProgress 非空（事件到达即显示；安装结束清空即隐藏并归零，同旧 finally）

import { useTranslation } from "react-i18next";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { fmtTs } from "@/shared/lib/format";
import { openRepo } from "@/shared/lib/links";
import { BTN } from "@/shared/lib/ui";

export function AboutSection() {
  const { t } = useTranslation();
  const appVersion = useAppStore((s) => s.appVersion);
  const updaterHealth = useAppStore((s) => s.updaterHealth);
  const updaterHealthError = useAppStore((s) => s.updaterHealthError);
  const updateInfo = useAppStore((s) => s.updateInfo);
  const updateBusyKind = useAppStore((s) => s.updateBusyKind);
  const updateLastCheckAt = useAppStore((s) => s.updateLastCheckAt);
  const updateCheckError = useAppStore((s) => s.updateCheckError);
  const downloadProgress = useAppStore((s) => s.downloadProgress);
  const installPendingUpdate = useAppStore((s) => s.installPendingUpdate);
  const toast = useAppStore((s) => s.toast);

  const openHelp = async (target: "docs" | "template") => {
    try {
      const paths = await cmd.getUpdaterHelpPaths();
      await openUrl(target === "docs" ? paths.docsPath : paths.templatePath);
    } catch (e) {
      toast(t("Failed to open help: {{error}}", { error: String(e) }), "error");
    }
  };

  // 更新源健康徽标：就绪 / 检测中 / 异常（原因在卡片内展开）
  const healthBadge = updaterHealthError
    ? { cls: "failed", text: t("Error") }
    : updaterHealth === null
      ? { cls: "starting", text: t("Checking...") }
      : updaterHealth.configured
        ? { cls: "running", text: t("Ready") }
        : { cls: "failed", text: t("Error") };
  const healthProblem = updaterHealthError !== null || (updaterHealth !== null && !updaterHealth.configured);
  const healthDetail = updaterHealthError
    ? t("Check failed: {{error}}", { error: updaterHealthError })
    : updaterHealth && !updaterHealth.configured
      ? updaterHealth.message
      : null;

  const updateBtnText =
    updateBusyKind === "check"
      ? t("Checking...")
      : updateBusyKind === "install"
        ? t("Updating...")
        : updateInfo
          ? t("Update Now")
          : t("Check for Updates");

  const notes = updateInfo?.releaseNotes?.trim() ?? "";
  const p = downloadProgress;
  const progressText = p
    ? p.stage === "restarting"
      ? t("Installation complete, restarting…")
      : p.stage === "installing"
        ? t("Installing…")
        : p.stage === "retrying"
          ? t("Download failed, retrying ({{attempt}}/{{max}})…", { attempt: p.attempt, max: p.maxAttempts })
          : p.percent !== null
            ? t("Downloading v{{version}}: {{percent}}%", { version: p.version, percent: Math.floor(p.percent) })
            : t("Downloading v{{version}}: {{mb}} MB", { version: p.version, mb: (p.downloadedBytes / 1024 / 1024).toFixed(1) })
    : "";
  const progressWidth = p
    ? p.stage === "restarting" || p.stage === "installing"
      ? "100%"
      : p.percent !== null
        ? `${p.percent}%`
        : undefined
    : "0%";

  return (
    <section className="settings-section" id="section-about">
      <h2 className="mb-4 text-base font-semibold">{t("About")}</h2>

      <div className="flex items-start gap-4 border-b border-border py-3">
        <span className="w-36 shrink-0 text-sm font-medium">{t("App Version")}</span>
        <span className="font-mono text-sm" id="about-version">{appVersion}</span>
      </div>

      {/* 更新状态聚合卡 */}
      <div className="mt-3 flex max-w-2xl flex-col gap-3 rounded-xl border border-border bg-card p-4 text-card-foreground">
        <div className="flex items-center justify-between gap-3">
          <div className="text-sm font-medium">{t("Updates")}</div>
          <span className={`status-badge ${healthBadge.cls}`}>
            <span className="dot"></span>
            <span>{healthBadge.text}</span>
          </span>
        </div>

        {healthProblem && healthDetail && (
          <div className="text-xs text-destructive">
            {healthDetail}
            <span className="ml-2">
              {t("Configuration Help")}:{" "}
              <a className="cursor-pointer text-primary underline-offset-4 hover:underline" onClick={() => void openHelp("docs")}>
                {t("Setup Guide")}
              </a>
              {" · "}
              <a className="cursor-pointer text-primary underline-offset-4 hover:underline" onClick={() => void openHelp("template")}>
                {t("Config Template")}
              </a>
            </span>
          </div>
        )}

        <div className="text-sm">
          {updateInfo?.hasUpdate && updateInfo.availableVersion ? (
            <span>
              v{updateInfo.currentVersion} →{" "}
              <span className="font-medium text-primary">v{updateInfo.availableVersion}</span>
            </span>
          ) : updateCheckError ? (
            <span className="text-destructive">{t("Check failed: {{error}}", { error: updateCheckError })}</span>
          ) : updateLastCheckAt !== null ? (
            <span>{t("Already up to date")}</span>
          ) : (
            <span className="opacity-60">{t("Checking...")}</span>
          )}
        </div>
        {notes && <div className="text-xs whitespace-pre-wrap opacity-70">{notes}</div>}
        {updateLastCheckAt !== null && (
          <div className="text-xs opacity-50">
            {t("Last checked {{at}}", { at: fmtTs(Math.floor(updateLastCheckAt / 1000)) })}
          </div>
        )}

        <div className="flex items-center gap-3">
          <button className={BTN} id="btn-check-update" disabled={updateBusyKind !== null} onClick={() => void installPendingUpdate()}>
            {updateBtnText}
          </button>
        </div>

        {p && (
          <div className="flex items-center gap-3">
            <span className="shrink-0 text-xs font-medium">{t("Update Progress")}</span>
            <div className="update-progress-track">
              <div className="update-progress-bar" style={progressWidth ? { width: progressWidth } : undefined}></div>
            </div>
            <span className="text-xs">{progressText}</span>
          </div>
        )}
      </div>

      <div className="mt-3 flex items-start gap-4 border-t border-border py-3">
        <span className="w-36 shrink-0 text-sm font-medium">GitHub</span>
        <a className="cursor-pointer text-sm text-primary underline-offset-4 hover:underline" onClick={() => void openRepo()}>
          {t("Open in Browser")}
        </a>
      </div>
    </section>
  );
}
