// 更新徽标：检测到更新时出现在 header 软件名右侧的圆形箭头按钮（幽灵样式，
// 绿色随亮暗模式——参照 cc-switch 的 green-600/green-400，见 style.css .update-badge）。
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
      className="update-badge relative inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-60"
      title={t("Update to {{version}}", { version: updateInfo.availableVersion })}
      aria-label={t("Update Now")}
      disabled={busyKind !== null}
      onClick={() => void installPendingUpdate()}
    >
      <svg className="absolute inset-0 h-full w-full -rotate-90" viewBox="0 0 20 20" aria-hidden="true">
        {clamped !== null && (
          <>
            <circle cx="10" cy="10" r={R} fill="none" stroke="currentColor" strokeOpacity="0.25" strokeWidth="2" />
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
          </>
        )}
      </svg>
      {/* lucide ArrowUpCircle（与 cc-switch 更新入口同款） */}
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10" />
        <path d="m16 12-4-4-4 4" />
        <path d="M12 16V8" />
      </svg>
    </button>
  );
}
