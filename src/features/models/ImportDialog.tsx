// features/models/ImportDialog：从本机其他 agent 工具导入供应商声明。
// 凭据语义与 Codex 一致：只带走环境变量引用（`env`），来源持明文密钥的
// 条目只提示、不读取值。已存在的路由由后端跳过，不覆盖用户现有配置。

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Badge } from "@/shared/components/ui/badge";
import { Button } from "@/shared/components/ui/button";
import { Switch } from "@/shared/components/ui/switch";
import { Modal } from "@/shared/components/Modal";
import * as cmd from "@/shared/commands";
import type { ImportGroup } from "@/shared/types";

/**
 * 来源标识 → 展示名。显式映射而非查表：字典 key 是原文，
 * 索引出来的 string 会绕过 i18n 的类型检查。
 */
function sourceLabel(source: string): "Codex" | "Claude Code" | "opencode" | "Environment variables" {
  switch (source) {
    case "claude-code":
      return "Claude Code";
    case "opencode":
      return "opencode";
    case "env":
      return "Environment variables";
    default:
      return "Codex";
  }
}

export function ImportDialog({
  open,
  onClose,
  onImported,
}: {
  open: boolean;
  onClose: () => void;
  onImported: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [groups, setGroups] = useState<ImportGroup[]>([]);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [running, setRunning] = useState(false);

  useEffect(() => {
    if (!open) return;
    setSelected(new Set());
    setLoading(true);
    cmd
      .modelImportScan()
      .then(setGroups)
      .catch((e) => toast.error(t("Operation failed: {{error}}", { error: String(e) })))
      .finally(() => setLoading(false));
  }, [open, t]);

  const total = useMemo(
    () => groups.reduce((sum, g) => sum + g.entries.length, 0),
    [groups],
  );

  const toggle = (key: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const run = async () => {
    if (selected.size === 0) return;
    setRunning(true);
    try {
      const result = await cmd.modelImportRun([...selected]);
      toast.success(
        t("Imported {{imported}} provider(s); {{skipped}} skipped.", {
          imported: result.imported,
          skipped: result.skipped,
        }),
      );
      if (result.literal > 0) {
        toast.info(
          t("{{count}} source(s) hold a literal key; set an environment variable for them.", {
            count: result.literal,
          }),
        );
      }
      await onImported();
      onClose();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
    } finally {
      setRunning(false);
    }
  };

  return (
    <Modal
      open={open}
      onRequestClose={onClose}
      cardStyle={{ width: 640 }}
      title={t("Import providers")}
    >
      <h3 className="text-sm font-semibold">{t("Import providers")}</h3>
      <p className="text-xs text-muted-foreground">
        {t("Scan other agent tools on this machine and reuse their provider declarations.")}{" "}
        {t("Only environment variable references are imported; literal keys are never read.")}
      </p>

      {loading ? (
        <p className="text-xs text-muted-foreground">{t("Scanning…")}</p>
      ) : total === 0 ? (
        <p className="text-xs text-muted-foreground">
          {t("Nothing to import was found on this machine.")}
        </p>
      ) : (
        <div className="max-h-80 overflow-y-auto rounded-lg border border-border">
          {groups
            .filter((g) => g.entries.length > 0)
            .map((group) => (
              <div key={group.source} className="border-b border-border last:border-b-0">
                <p className="px-3 py-2 text-xs font-medium text-muted-foreground">
                  {t(sourceLabel(group.source))}
                </p>
                {group.entries.map((entry) => (
                  <label
                    key={entry.key}
                    className="flex cursor-pointer items-start gap-2 px-3 py-2 hover:bg-muted"
                  >
                    <Switch
                      className="mt-0.5"
                      checked={selected.has(entry.key)}
                      onCheckedChange={() => toggle(entry.key)}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-2 text-sm">
                        {entry.name}
                        <Badge variant="secondary" className="font-mono">
                          {entry.route}
                        </Badge>
                        {entry.wireApi && (
                          <Badge variant="secondary" className="font-mono">
                            {entry.wireApi}
                          </Badge>
                        )}
                      </span>
                      {entry.baseURL && (
                        <span className="block truncate font-mono text-xs tabular-nums text-muted-foreground">
                          {entry.baseURL}
                        </span>
                      )}
                      <span className="block text-xs text-muted-foreground">
                        {entry.credential === "env"
                          ? t("Env var: {{name}}", { name: entry.envKey ?? "" })
                          : entry.credential === "literal"
                            ? t("Holds a literal key; set an environment variable instead.")
                            : t("No credentials declared")}
                        {entry.models.length > 0 &&
                          ` · ${t("{{count}} models", { count: entry.models.length })}`}
                      </span>
                    </span>
                  </label>
                ))}
              </div>
            ))}
        </div>
      )}

      <div className="flex justify-end gap-2">
        <Button variant="outline" onClick={onClose}>
          {t("Cancel")}
        </Button>
        <Button onClick={() => void run()} disabled={selected.size === 0 || running}>
          {running
            ? t("Importing…")
            : t("Import {{count}} selected", { count: selected.size })}
        </Button>
      </div>
    </Modal>
  );
}
