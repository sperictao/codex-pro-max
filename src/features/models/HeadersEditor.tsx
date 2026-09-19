// features/models/HeadersEditor：请求头键值编辑器。
// 静态头（http_headers）与环境变量头（env_http_headers）共用一套行编辑，
// 差别只在值那一列的提示文案——两者的值都是"字符串"，但语义不同：
// 前者是字面量，后者是变量名。空键的行在下发前被丢弃。

import { PlusIcon, Trash2Icon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/shared/components/ui/button";
import { Input } from "@/shared/components/ui/input";

export type HeaderRow = { key: string; value: string };

export function rowsFromRecord(record: Record<string, string> | undefined): HeaderRow[] {
  return Object.entries(record ?? {}).map(([key, value]) => ({ key, value }));
}

export function recordFromRows(rows: HeaderRow[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (!key) continue;
    out[key] = row.value.trim();
  }
  return out;
}

export function HeadersEditor({
  id,
  label,
  hint,
  keyPlaceholder,
  valuePlaceholder,
  rows,
  onChange,
  className,
}: {
  /** 用于把 label 关联到第一行输入，保证可访问名唯一 */
  id: string;
  label: string;
  hint?: string;
  keyPlaceholder: string;
  valuePlaceholder: string;
  rows: HeaderRow[];
  onChange: (rows: HeaderRow[]) => void;
  className?: string;
}) {
  const { t } = useTranslation();

  const update = (index: number, patch: Partial<HeaderRow>) => {
    onChange(rows.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  };

  return (
    <div className={className}>
      <div className="mb-1 flex items-center justify-between gap-2">
        <label className="text-xs font-medium" htmlFor={`${id}-key-0`}>
          {label}
        </label>
        <Button
          type="button"
          variant="ghost"
          size="xs"
          onClick={() => onChange([...rows, { key: "", value: "" }])}
        >
          <PlusIcon />
          {t("Add header")}
        </Button>
      </div>
      {hint && <p className="mb-1.5 text-xs text-muted-foreground">{hint}</p>}
      {rows.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t("No headers configured")}</p>
      ) : (
        <div className="flex flex-col gap-1.5">
          {rows.map((row, index) => (
            <div key={index} className="flex items-center gap-1.5">
              <Input
                id={`${id}-key-${index}`}
                className="font-mono tabular-nums"
                placeholder={keyPlaceholder}
                aria-label={t("Header name")}
                value={row.key}
                onChange={(e) => update(index, { key: e.target.value })}
              />
              <Input
                className="font-mono tabular-nums"
                placeholder={valuePlaceholder}
                aria-label={t("Header value")}
                value={row.value}
                onChange={(e) => update(index, { value: e.target.value })}
              />
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={t("Remove header")}
                onClick={() => onChange(rows.filter((_, i) => i !== index))}
              >
                <Trash2Icon />
              </Button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
