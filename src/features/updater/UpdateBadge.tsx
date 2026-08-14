// 更新徽标：检测到更新时出现在 header 软件名右侧的幽灵圆形箭头按钮
// （绿色随亮暗模式——参照 cc-switch 的 green-600/green-400，见 style.css .update-badge）。
// 点击立即安装；仅下载期间按钮外沿出现进度环（兼作按钮边框），从 12 点方向
// 顺时针填充（installing/restarting 视为满环），无进度时无圆环。

import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";

// 圆环贴合按钮边缘：viewBox 20、r=9、stroke-width 2 → 描边外沿即按钮边界
const R = 9;
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
      {/* 进度环仅在下载时出现（此时它同时充当按钮边框）：轨道 + 顺时针进度弧 */}
      {clamped !== null && (
        <svg className="absolute inset-0 h-full w-full -rotate-90" viewBox="0 0 20 20" aria-hidden="true">
          <circle cx="10" cy="10" r={R} fill="none" stroke="currentColor" strokeOpacity="0.3" strokeWidth="2" />
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
        </svg>
      )}
      {/* lucide ArrowUp（无圆圈本体，圆环由上方边框承担） */}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M12 19V5" />
        <path d="m5 12 7-7 7 7" />
      </svg>
    </button>
  );
}
