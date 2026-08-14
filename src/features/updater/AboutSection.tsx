// 关于分区（updater 域）：版本、更新源健康、检查/安装更新、下载进度、GitHub 链接
// 进度行可见性 = store.downloadProgress 非空（事件到达即显示；安装结束清空即隐藏并归零，同旧 finally）

import { useTranslation } from "react-i18next";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { BTN } from "@/shared/lib/ui";

export function AboutSection() {
  const { t } = useTranslation();
  const appVersion = useAppStore((s) => s.appVersion);
  const updaterHealth = useAppStore((s) => s.updaterHealth);
  const updaterHealthError = useAppStore((s) => s.updaterHealthError);
  const updateInfo = useAppStore((s) => s.updateInfo);
  const updateBusyKind = useAppStore((s) => s.updateBusyKind);
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
  const openGithub = async () => {
    try {
      await openUrl("https://github.com/sperictao/codex-pro-max");
    } catch (e) {
      toast(t("Failed to open link: {{error}}", { error: String(e) }), "error");
    }
  };

  // 健康单元格：错误 / 检测中 / 就绪 / 未就绪原因
  const healthText = updaterHealthError
    ? t("Check failed: {{error}}", { error: updaterHealthError })
    : updaterHealth === null
      ? t("Checking...")
      : updaterHealth.configured
        ? t("Ready")
        : updaterHealth.message;
  const healthCls = updaterHealthError
    ? "err"
    : updaterHealth === null
      ? ""
      : updaterHealth.configured
        ? "ok"
        : "err";
  const helpVisible = updaterHealthError !== null || (updaterHealth !== null && !updaterHealth.configured);

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

      <div className="flex items-start gap-4 border-b border-border py-3">
        <span className="w-36 shrink-0 text-sm font-medium">{t("Update Source Status")}</span>
        {/* 旧实现检测后整替换 className 为 health-status ok/err（丢掉 text-sm），初始静态为 text-sm */}
        <span className={healthCls ? `health-status ${healthCls}` : "text-sm"}>{healthText}</span>
      </div>

      {helpVisible && (
        <div className="flex items-start gap-4 border-b border-border py-3" id="updater-help-row">
          <span className="w-36 shrink-0 text-sm font-medium">{t("Configuration Help")}</span>
          <span className="text-sm">
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

      <div className="flex items-start gap-4 border-b border-border py-3">
        <span className="w-36 shrink-0 text-sm font-medium"></span>
        <button className={BTN} id="btn-check-update" disabled={updateBusyKind !== null} onClick={() => void installPendingUpdate()}>
          {updateBtnText}
        </button>
      </div>

      {updateInfo?.hasUpdate && updateInfo.availableVersion && (
        <div className="flex items-start gap-4 border-b border-border py-3" id="update-available-row">
          <span className="w-36 shrink-0 text-sm font-medium">{t("Available Update")}</span>
          <div className="flex flex-col gap-1">
            <span className="font-mono text-sm">{`v${updateInfo.availableVersion}`}</span>
            {notes && <span className="text-xs whitespace-pre-wrap opacity-70">{notes}</span>}
          </div>
        </div>
      )}

      {p && (
        <div className="flex items-start gap-4 border-b border-border py-3" id="update-progress-row">
          <span className="w-36 shrink-0 text-sm font-medium">{t("Update Progress")}</span>
          <div className="flex items-center gap-3">
            <div className="update-progress-track">
              <div className="update-progress-bar" style={progressWidth ? { width: progressWidth } : undefined}></div>
            </div>
            <span className="text-xs">{progressText}</span>
          </div>
        </div>
      )}

      <div className="flex items-start gap-4 py-3">
        <span className="w-36 shrink-0 text-sm font-medium">GitHub</span>
        <a className="cursor-pointer text-sm text-primary underline-offset-4 hover:underline" onClick={() => void openGithub()}>
          {t("Open in Browser")}
        </a>
      </div>
    </section>
  );
}
