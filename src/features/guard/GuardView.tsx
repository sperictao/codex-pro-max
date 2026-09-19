// 看守视图：分组参数卡渲染（旧 guard.ts renderGuardView 的 JSX 化）。
// 挂载即强制刷新视图 + 文件列表（旧 btn-guard 点击行为）；3s 轮询由 App 发起、store 内做增量/焦点判断。

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { HelpCircleIcon, MoreHorizontalIcon } from "lucide-react";
import { useAppStore } from "@/shared/store";
import { fmtTs } from "@/shared/lib/format";
import { Button } from "@/shared/components/ui/button";
import { Card, CardTitle } from "@/shared/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/components/ui/dropdown-menu";
import { Input } from "@/shared/components/ui/input";
import { Switch } from "@/shared/components/ui/switch";
import { Textarea } from "@/shared/components/ui/textarea";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/components/ui/tooltip";
import type { GuardGroupView, GuardParamView } from "@/shared/types";
import { AddParamModal } from "./AddParamModal";
import * as ops from "./ops";

const LOCK_SVG = (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><rect width="18" height="11" x="3" y="11" rx="2" ry="2" /><path d="M7 11V7a5 5 0 0 1 10 0v4" /></svg>
);
const UNLOCK_SVG = (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><rect width="18" height="11" x="3" y="11" rx="2" ry="2" /><path d="M7 11V7a5 5 0 0 1 9.9-1" /></svg>
);

const STATUS_MAP: Record<string, { key: string; cls: string }> = {
  match: { key: "Match", cls: "running" },
  drift: { key: "Drift", cls: "failed" },
  missing: { key: "Missing", cls: "starting" },
  error: { key: "Error", cls: "failed" },
};

// 文本/数字编辑器为非受控：key 随已存值变化（外部更新→重挂载显示新值）；
// blur 且值有变化才落盘——复刻旧「change 事件（失焦且修改过）才 guard_set_value」语义
function ParamEditor({ p }: { p: GuardParamView }) {
  const { t } = useTranslation();
  if (p.valueType === "bool") {
    return (
      <div className="flex items-center gap-2">
        <Switch data-guard-id={p.id} checked={p.value === true} disabled={p.locked}
          onCheckedChange={(next) => void ops.toggleBool(p.id, next)} />
        <span className="text-xs tabular-nums text-muted-foreground">
          {p.value === true ? "true" : "false"} {t("(recommended {{default}})", { default: String(p.default) })}
        </span>
      </div>
    );
  }
  if (p.valueType === "int" || p.valueType === "string") {
    const saved = String(p.value ?? "");
    return (
      <Input type={p.valueType === "int" ? "number" : "text"} className="font-mono tabular-nums" data-guard-id={p.id}
        key={`${p.id}:${saved}`} disabled={p.locked} defaultValue={saved}
        onBlur={(e) => { if (e.target.value !== saved) void ops.setValue(p.id, e.target.value); }} />
    );
  }
  if (p.valueType === "text") {
    const saved = String(p.value ?? "");
    // field-sizing-fixed：保留旧的固定 96px 高度 + 内部滚动，不随文本增长撑开参数卡
    return (
      <Textarea className="min-h-24 font-mono field-sizing-fixed" data-guard-id={p.id} key={`${p.id}:${saved}`}
        disabled={p.locked} defaultValue={saved}
        onBlur={(e) => { if (e.target.value !== saved) void ops.setValue(p.id, e.target.value); }} />
    );
  }
  return (
    <span className="text-xs text-muted-foreground">
      {t("No editable value; applying performs \"{{action}}\"", { action: t(p.applyMode === "toml_absent" ? "delete" : "write") })}
    </span>
  );
}

function ParamCard({ p }: { p: GuardParamView }) {
  const { t } = useTranslation();
  const s = STATUS_MAP[p.status] ?? STATUS_MAP.error;
  return (
    <Card className="guard-param-card gap-0 rounded-lg p-3">
      <div className="flex items-center justify-between">
        <span className="inline-flex items-center gap-1 text-sm font-medium">
          {p.label}
          {(p.description || p.path) && (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon-xs" aria-label={p.description || p.path}>
                    <HelpCircleIcon className="size-3" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent className="flex-col items-start gap-0.5 text-left">
                  {p.description && <span>{p.description}</span>}
                  {p.path && <span className="font-mono text-[11px] break-all opacity-70">{p.path}</span>}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
        </span>
        <span className={`status-badge ${s.cls}`}><span className="dot"></span><span>{t(s.key)}</span></span>
      </div>
      <div className="mt-1 flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className={`guard-param-actual font-mono text-xs tabular-nums ${p.status === "match" ? "ok" : "bad"}`}>
            {t("Current: ")}{p.actual ?? p.error ?? t("Unknown")}
          </div>
          <div className="mt-2"><ParamEditor p={p} /></div>
          {p.locked && (
            <div className="mt-1 text-xs tabular-nums text-muted-foreground">
              {t("Last checked {{checked}} | Last auto-restored {{restored}}", {
                checked: fmtTs(p.lastChecked),
                restored: fmtTs(p.lastRestored),
              })}
            </div>
          )}
        </div>
        <span className="flex w-[30%] shrink-0 flex-row flex-wrap items-center justify-end gap-1 self-center">
          <Switch
            checked={p.applied} disabled={p.locked}
            title={p.applied ? t("Disable") : t("Enable")}
            aria-label={p.applied ? t("Disable") : t("Enable")}
            onCheckedChange={() => void ops.toggleApplied(p.id)} />
          {p.locked ? (
            <Button variant="outline" size="sm" onClick={() => void ops.setLocked(p.id, false)}>
              {LOCK_SVG}{t("Unlock")}
            </Button>
          ) : (
            <Button variant="outline" size="sm" disabled={!p.applied} onClick={() => void ops.setLocked(p.id, true)}>
              {UNLOCK_SVG}{t("Lock")}
            </Button>
          )}
          {!p.applied && !p.locked && (
            <Button variant="outline" size="sm" onClick={() => void ops.removeConfig(p.id)}>
              {t("Remove Config")}
            </Button>
          )}
          {p.custom && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon-sm" aria-label={t("More actions")}><MoreHorizontalIcon /></Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem variant="destructive" onSelect={() => void ops.removeCustom(p.id)}>
                  {t("Delete")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </span>
      </div>
    </Card>
  );
}

function GroupCard({ g, onAddParam }: { g: GuardGroupView; onAddParam: (fileId: string) => void }) {
  const { t } = useTranslation();
  return (
    <Card className="gap-0 p-4 shadow-xs" data-group-id={g.id}>
      <CardTitle className="text-sm font-semibold">{g.name}</CardTitle>
      <div className="mb-2 font-mono text-xs text-muted-foreground">~/.codex/{g.file}</div>
      {g.error && <div className="mb-2 text-xs text-destructive">{g.error}</div>}
      <div className="flex flex-col gap-2">
        {g.params.map((p) => <ParamCard key={p.id} p={p} />)}
      </div>
      <div className="mt-2">
        <Button variant="outline" size="sm" onClick={() => onAddParam(g.id)}>{t("+ Add Parameter")}</Button>
      </div>
    </Card>
  );
}

export function GuardView() {
  const { t } = useTranslation();
  const guardView = useAppStore((s) => s.guardView);
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [addParamFileId, setAddParamFileId] = useState<string | null>(null);

  // 进入视图：强制刷新视图 + 文件列表（旧 btn-guard 点击行为）
  useEffect(() => {
    void useAppStore.getState().refreshGuardView(true);
    void ops.refreshFiles();
  }, []);

  const openAddFor = (fileId: string | null) => {
    setAddParamFileId(fileId);
    setAddModalOpen(true);
  };

  return (
    <main className="app-page-scroll flex-1 overflow-y-auto" id="guard-view">
      <h2 className="mb-2 text-base font-semibold">{t("Config Guard")}</h2>
      <p className="mb-4 max-w-3xl text-xs leading-5 text-muted-foreground">
        {t("Apply = write the parameter value into its file (auto-backup to")}{" "}
        <code className="rounded bg-muted px-1 font-mono">~/.codex/dashi-backups/</code>{" "}
        {t("before writing); Lock = verify every 60 seconds and revert drift automatically. Locked parameters are read-only; unlock before editing. The master switch and file management are in Settings → Guard.")}
      </p>

      <div className="flex flex-col gap-4" id="guard-groups">
        {guardView?.groups.map((g) => <GroupCard key={g.id} g={g} onAddParam={(id) => openAddFor(id)} />)}
      </div>

      <div className="mt-4">
        <Button
          variant="outline"
          className="w-full"
          id="guard-add-toggle"
          onClick={() => openAddFor(null)}
        >
          {t("+ Add Custom Parameter")}
        </Button>
      </div>
      <div className="mt-4 max-w-4xl text-center">
        <Button variant="link" size="xs" className="cursor-pointer" onClick={() => void ops.openSchemaFile()}>
          {t("Open schema file (manual editing for advanced users)")}
        </Button>
      </div>

      <AddParamModal
        open={addModalOpen}
        preferredFileId={addParamFileId}
        onClose={() => setAddModalOpen(false)}
      />
    </main>
  );
}
