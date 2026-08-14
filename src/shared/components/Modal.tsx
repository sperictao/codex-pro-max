import type { ReactNode } from "react";

// 应用内弹窗壳：复用旧 .modal-overlay/.modal-card 样式（主题感知）。
// onOverlayClick 仅在「点遮罩本身」时触发（旧 guard 弹窗的点遮罩关闭语义）；
// 不传则点遮罩无响应（旧 codex-restart 弹窗语义）。
export function Modal({
  open,
  onOverlayClick,
  labelledBy,
  cardClassName,
  children,
}: {
  open: boolean;
  onOverlayClick?: () => void;
  labelledBy?: string;
  cardClassName?: string;
  children: ReactNode;
}) {
  if (!open) return null;
  return (
    <div
      className="modal-overlay"
      onClick={(e) => {
        if (onOverlayClick && e.target === e.currentTarget) onOverlayClick();
      }}
    >
      <div className={`modal-card${cardClassName ? ` ${cardClassName}` : ""}`} role="dialog" aria-modal="true" aria-labelledby={labelledBy}>
        {children}
      </div>
    </div>
  );
}
