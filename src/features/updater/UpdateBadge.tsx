// 更新徽标：检测到更新时出现在 header 软件名右侧的圆形箭头按钮
// （绿色随亮暗模式——参照 cc-switch 的 green-600/green-400，见 style.css .update-badge）。
// 常态：1.4px 细实体环 + 细箭头（有效描边均 ≈1.4px）；点击立即安装。
// 下载中：环加粗为 2.8px 进度环（12 点顺时针填充），箭头切换为百分比数值；
// installing/restarting 视为满环（100）。

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

  const downloading = clamped !== null;

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
      {/* 常态：1.4px 细实体环；下载中：2.8px 进度环（淡轨道 + 顺时针进度弧） */}
      <svg className="absolute inset-0 h-full w-full -rotate-90" viewBox="0 0 20 20" aria-hidden="true">
        {downloading ? (
          <>
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
          </>
        ) : (
          <circle cx="10" cy="10" r={R} fill="none" stroke="currentColor" strokeWidth="1" data-idle-ring />
        )}
      </svg>
      {downloading ? (
        // 下载中：箭头切换为百分比数值
        <span className="text-[9px] leading-none font-semibold tabular-nums">{Math.round(clamped)}</span>
      ) : (
        /* lucide ArrowUp（无圆圈本体）；15px + strokeWidth 2.2 → 有效描边 ≈1.4px，与常态细环一致 */
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" data-arrow>
          <path d="M12 19V5" />
          <path d="m5 12 7-7 7 7" />
        </svg>
      )}
    </button>
  );
}
