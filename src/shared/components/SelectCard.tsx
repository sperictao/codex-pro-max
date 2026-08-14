import type { ReactNode } from "react";

const CHECK_SVG = (
  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3.5" strokeLinecap="round" strokeLinejoin="round">
    <path d="M20 6 9 17l-5-5" />
  </svg>
);

// 选择卡：语言/主题模式/主题族共用（旧 .select-card + 选中勾，选中态 .selected + aria-pressed）
export function SelectCard({
  selected,
  onClick,
  children,
}: {
  selected: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button type="button" className={`select-card${selected ? " selected" : ""}`} aria-pressed={selected} onClick={onClick}>
      <span className="select-card-check">{CHECK_SVG}</span>
      {children}
    </button>
  );
}
