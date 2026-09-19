// features/models/ModelPickerDialog：从 models.dev 目录里挑一个模型 id。
// 只负责"选一个 id"这一件事——容量与档位信息由调用方按 id 再查，避免
// 目录数据在两处各存一份。目录缺失时这里直接给出引导而不是空列表。

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SearchIcon } from "lucide-react";
import { Badge } from "@/shared/components/ui/badge";
import { Button } from "@/shared/components/ui/button";
import { Input } from "@/shared/components/ui/input";
import { Modal } from "@/shared/components/Modal";
import type { CatalogEntry, CatalogView } from "@/shared/types";
import { fmtTokens } from "./ops";

/** 目录条目按 provider 摊平，供跨路由搜索 */
function flatten(catalog: CatalogView): CatalogEntry[] {
  return Object.values(catalog.providers).flat();
}

export function ModelPickerDialog({
  open,
  catalog,
  route,
  onPick,
  onClose,
  onRefresh,
}: {
  open: boolean;
  catalog: CatalogView;
  /** 当前路由：优先展示它的模型，其余折叠在后 */
  route: string;
  onPick: (modelId: string) => void;
  onClose: () => void;
  onRefresh: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [refreshing, setRefreshing] = useState(false);

  const all = useMemo(() => flatten(catalog), [catalog]);
  const matched = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const matched = needle
      ? all.filter(
          (e) =>
            e.id.toLowerCase().includes(needle) || e.name.toLowerCase().includes(needle),
        )
      : all;
    // 当前路由的模型排前面：绝大多数时候用户就在这个路由里挑
    const current = matched.filter((e) => e.provider === route);
    const others = matched.filter((e) => e.provider !== route);
    return [...current, ...others].slice(0, 200);
  }, [all, query, route]);

  const refresh = async () => {
    setRefreshing(true);
    try {
      await onRefresh();
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <Modal
      open={open}
      onRequestClose={onClose}
      cardStyle={{ width: 560 }}
      title={t("Pick a model")}
    >
      <h3 className="text-sm font-semibold">{t("Pick a model")}</h3>

      {catalog.missing ? (
        <div className="flex flex-col gap-2">
          <p className="text-xs text-muted-foreground">
            {t("The model catalog has not been downloaded yet.")}
          </p>
          <p className="text-xs text-muted-foreground">
            {t("Download it to browse model ids with context and reasoning metadata.")}
          </p>
        </div>
      ) : (
        <>
          <div className="relative">
            <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              className="pl-8"
              placeholder={t("Search models")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          <div className="max-h-80 overflow-y-auto rounded-lg border border-border">
            {matched.length === 0 ? (
              <p className="p-3 text-xs text-muted-foreground">{t("No models match.")}</p>
            ) : (
              matched.map((entry) => (
                <button
                  key={`${entry.provider}:${entry.id}`}
                  type="button"
                  className="flex w-full items-center justify-between gap-3 border-b border-border px-3 py-2 text-left last:border-b-0 hover:bg-muted focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
                  onClick={() => {
                    onPick(entry.id);
                    onClose();
                  }}
                >
                  <span className="min-w-0">
                    <span className="block truncate text-sm">{entry.name}</span>
                    <span className="block truncate font-mono text-xs tabular-nums text-muted-foreground">
                      {entry.id} · {entry.provider}
                    </span>
                  </span>
                  <span className="flex shrink-0 items-center gap-1">
                    {entry.reasoning && (
                      <Badge variant="secondary">{t("reasoning")}</Badge>
                    )}
                    <span className="font-mono text-xs tabular-nums text-muted-foreground">
                      {fmtTokens(entry.context)}
                    </span>
                  </span>
                </button>
              ))
            )}
          </div>
        </>
      )}

      <div className="flex justify-end gap-2">
        <Button variant="outline" onClick={() => void refresh()} disabled={refreshing}>
          {refreshing ? t("Refreshing…") : t("Refresh catalog")}
        </Button>
        <Button variant="outline" onClick={onClose}>
          {t("Close")}
        </Button>
      </div>
    </Modal>
  );
}
