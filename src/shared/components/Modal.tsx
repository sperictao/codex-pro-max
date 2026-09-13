import type { CSSProperties, ReactNode } from "react";
import { Dialog, DialogContent, DialogTitle } from "@/shared/components/ui/dialog";

// 应用内弹窗壳（craft-spec 批 0）：底层换成 Radix Dialog，公开 API 与旧实现一致，
// 额外获得焦点陷阱、滚动锁、Escape 与焦点归还——旧实现只有 role/aria 属性。
// onOverlayClick 仅在「点遮罩本身」时触发；不传则点遮罩与 Escape 都不关闭（沿用旧语义）。
export function Modal({
  open,
  onOverlayClick,
  labelledBy,
  title,
  cardClassName,
  cardStyle,
  children,
}: {
  open: boolean;
  onOverlayClick?: () => void;
  labelledBy?: string;
  /** 无障碍标题：渲染为 sr-only 的 DialogTitle；可见标题仍由调用方自己渲染 */
  title?: string;
  cardClassName?: string;
  cardStyle?: CSSProperties;
  children: ReactNode;
}) {
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onOverlayClick?.();
      }}
    >
      <DialogContent
        className={cardClassName}
        style={cardStyle}
        aria-labelledby={labelledBy}
        showCloseButton={false}
        onInteractOutside={(e) => {
          if (!onOverlayClick) e.preventDefault();
        }}
        onEscapeKeyDown={(e) => {
          if (!onOverlayClick) e.preventDefault();
        }}
      >
        <DialogTitle className="sr-only">{title ?? ""}</DialogTitle>
        {children}
      </DialogContent>
    </Dialog>
  );
}
