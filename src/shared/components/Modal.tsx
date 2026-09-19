import type { CSSProperties, ReactNode } from "react";
import { Dialog, DialogContent, DialogTitle } from "@/shared/components/ui/dialog";

// 应用内弹窗壳（craft-spec 批 0）：底层换成 Radix Dialog，公开 API 保持轻量，
// 额外获得焦点陷阱、滚动锁、Escape 与焦点归还。onRequestClose 表示任意关闭请求
//（点击遮罩或 Escape）；不传则两种关闭方式都被阻止，供必须显式确认的弹窗使用。
export function Modal({
  open,
  onRequestClose,
  labelledBy,
  title,
  cardClassName,
  cardStyle,
  children,
}: {
  open: boolean;
  onRequestClose?: () => void;
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
        if (!next) onRequestClose?.();
      }}
    >
      <DialogContent
        className={cardClassName}
        style={cardStyle}
        aria-labelledby={labelledBy}
        showCloseButton={false}
        onInteractOutside={(e) => {
          if (!onRequestClose) e.preventDefault();
        }}
        onEscapeKeyDown={(e) => {
          if (!onRequestClose) e.preventDefault();
        }}
      >
        <DialogTitle className="sr-only">{title ?? ""}</DialogTitle>
        {children}
      </DialogContent>
    </Dialog>
  );
}
