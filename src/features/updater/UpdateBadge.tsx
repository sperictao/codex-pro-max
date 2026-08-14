// 更新徽标：检测到更新时出现在 header 软件名右侧的绿色圆形按钮。
// 点击立即安装；下载期间外圈圆环按 percent 顺时针填充（installing/restarting 视为满环）。

import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";

const R = 8;
const CIRCUMFERENCE = 2 * Math.PI * R;

export function UpdateBadge() {
  const { t } = useTranslation();
  const updateInfo = useAppStore((s) => s.updateInfo);
  const busyKind = useAppStore((s) => s.updateBusyKind);
  const progress = useAppStore((s) => s.downloadProgress);
  const installPendingUpdate = useAppStore((s) => s.installPendingUpdate);

  if (!updateInfo?.hasUpdate || !updateInfo.availableVersion) return null;

  const percent =
    progress?.percent ??
    (progress && (progress.stage === "installing" || progress.stage === "restarting") ? 100 : null);
  const clamped = percent === null ? null : Math.min(100, Math.max(0, percent));

  return (
    <button
      type="button"
      data-testid="update-badge"
      className="relative inline-flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded-full bg-green-500 text-white transition-colors hover:bg-green-600 disabled:cursor-not-allowed disabled:opacity-60"
      title={t("Update to {{version}}", { version: updateInfo.availableVersion })}
      aria-label={t("Update Now")}
      disabled={busyKind !== null}
      onClick={() => void installPendingUpdate()}
    >
      <svg className="absolute inset-0 h-full w-full -rotate-90" viewBox="0 0 20 20" aria-hidden="true">
        <circle cx="10" cy="10" r={R} fill="none" stroke="currentColor" strokeOpacity="0.35" strokeWidth="2" />
        {clamped !== null && (
          <circle
            cx="10"
            cy="10"
            r={R}
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeDasharray={CIRCUMFERENCE}
            strokeDashoffset={CIRCUMFERENCE * (1 - clamped / 100)}
            data-progress-ring
          />
        )}
      </svg>
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
        <polyline points="7 10 12 15 17 10" />
        <line x1="12" y1="15" x2="12" y2="3" />
      </svg>
    </button>
  );
}
