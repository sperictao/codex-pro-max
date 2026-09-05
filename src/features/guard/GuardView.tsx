// 看守视图：分组参数卡渲染（旧 guard.ts renderGuardView 的 JSX 化）。
// 挂载即强制刷新视图 + 文件列表（旧 btn-guard 点击行为）；3s 轮询由 App 发起、store 内做增量/焦点判断。

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";
import { fmtTs } from "@/shared/lib/format";
import { BTN_DANGER_SM, BTN_SM, INPUT_MONO, TOGGLE } from "@/shared/lib/ui";
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
        <input type="checkbox" className={TOGGLE} data-guard-id={p.id}
          checked={p.value === true} disabled={p.locked}
          onChange={(e) => void ops.toggleBool(p.id, e.target.checked)} />
        <span className="text-xs opacity-70">
          {p.value === true ? "true" : "false"} {t("(recommended {{default}})", { default: String(p.default) })}
        </span>
      </div>
    );
  }
  if (p.valueType === "int" || p.valueType === "string") {
    const saved = String(p.value ?? "");
    return (
      <input type={p.valueType === "int" ? "number" : "text"} className={INPUT_MONO} data-guard-id={p.id}
        key={`${p.id}:${saved}`} disabled={p.locked} defaultValue={saved}
        onBlur={(e) => { if (e.target.value !== saved) void ops.setValue(p.id, e.target.value); }} />
    );
  }
  if (p.valueType === "text") {
    const saved = String(p.value ?? "");
    return (
      <textarea className="guard-textarea" data-guard-id={p.id} key={`${p.id}:${saved}`}
        disabled={p.locked} defaultValue={saved}
        onBlur={(e) => { if (e.target.value !== saved) void ops.setValue(p.id, e.target.value); }} />
    );
  }
  return (
    <span className="text-xs opacity-60">
      {t("No editable value; applying performs \"{{action}}\"", { action: t(p.applyMode === "toml_absent" ? "delete" : "write") })}
    </span>
  );
}

function ParamCard({ p }: { p: GuardParamView }) {
  const { t } = useTranslation();
  const s = STATUS_MAP[p.status] ?? STATUS_MAP.error;
  return (
    <div className="guard-param-card rounded-lg border border-border bg-card text-card-foreground p-3">
      <div className="flex items-center justify-between">
        <span className="text-sm font-medium">
          {p.label}
          {(p.description || p.path) && (
            <>
              {" "}
              <span className="guard-param-help" tabIndex={0}>
                ?
                <span className="guard-param-desc">
                  {p.description && <span>{p.description}</span>}
                  {p.path && <span className="guard-param-desc-path">{p.path}</span>}
                </span>
              </span>
            </>
          )}
        </span>
        <span className={`status-badge ${s.cls}`}><span className="dot"></span><span>{t(s.key)}</span></span>
      </div>
      <div className="mt-1 flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className={`guard-param-actual font-mono text-xs ${p.status === "match" ? "ok" : "bad"}`}>
            {t("Current: ")}{p.actual ?? p.error ?? t("Unknown")}
          </div>
          <div className="mt-2"><ParamEditor p={p} /></div>
          {p.locked && (
            <div className="mt-1 text-xs opacity-50">
              {t("Last checked {{checked}} | Last auto-restored {{restored}}", {
                checked: fmtTs(p.lastChecked),
                restored: fmtTs(p.lastRestored),
              })}
            </div>
          )}
        </div>
        <span className="guard-param-actions flex w-[30%] shrink-0 flex-row flex-wrap items-center justify-end gap-1 self-center">
          <input type="checkbox" className="text-switch"
            data-state-text={p.applied ? t("Enabled") : t("Disabled")}
            checked={p.applied} disabled={p.locked}
            title={p.applied ? t("Disable") : t("Enable")}
            aria-label={p.applied ? t("Disable") : t("Enable")}
            onChange={() => void ops.toggleApplied(p.id)} />
          {p.locked ? (
            <button className={BTN_SM} onClick={() => void ops.setLocked(p.id, false)}>
              {LOCK_SVG}{t("Unlock")}
            </button>
          ) : (
            <button className={BTN_SM} disabled={!p.applied} onClick={() => void ops.setLocked(p.id, true)}>
              {UNLOCK_SVG}{t("Lock")}
            </button>
          )}
          {!p.applied && !p.locked && (
            <button className={BTN_SM} onClick={() => void ops.removeConfig(p.id)}>
              {t("Remove Config")}
            </button>
          )}
          {p.custom && (
            <button className={BTN_DANGER_SM} title={t("Delete custom parameter")} onClick={() => void ops.removeCustom(p.id)}>
              {t("Delete")}
            </button>
          )}
        </span>
      </div>
    </div>
  );
}

function GroupCard({ g, onAddParam }: { g: GuardGroupView; onAddParam: (fileId: string) => void }) {
  const { t } = useTranslation();
  return (
    <div className="rounded-xl border border-border bg-card text-card-foreground p-4" data-group-id={g.id}>
      <div className="text-sm font-semibold">{g.name}</div>
      <div className="mb-2 font-mono text-xs opacity-50">~/.codex/{g.file}</div>
      {g.error && <div className="mb-2 text-xs text-destructive">{g.error}</div>}
      <div className="flex flex-col gap-2">
        {g.params.map((p) => <ParamCard key={p.id} p={p} />)}
      </div>
      <div className="mt-2">
        <button className={BTN_SM} onClick={() => onAddParam(g.id)}>{t("+ Add Parameter")}</button>
      </div>
    </div>
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
    <main className="flex-1 overflow-y-auto p-6" id="guard-view">
      <h2 className="mb-2 text-base font-semibold">{t("Config Guard")}</h2>
      <p className="mb-4 max-w-3xl text-xs leading-5 opacity-60">
        {t("Apply = write the parameter value into its file (auto-backup to")}{" "}
        <code className="rounded bg-muted px-1 font-mono">~/.codex/dashi-backups/</code>{" "}
        {t("before writing); Lock = verify every 60 seconds and revert drift automatically. Locked parameters are read-only; unlock before editing. The master switch and file management are in Settings → Guard.")}
      </p>

      <div className="flex flex-col gap-4" id="guard-groups">
        {guardView?.groups.map((g) => <GroupCard key={g.id} g={g} onAddParam={(id) => openAddFor(id)} />)}
      </div>

      <div className="mt-4">
        <button
          className="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-9 w-full px-4"
          id="guard-add-toggle"
          onClick={() => openAddFor(null)}
        >
          <span>{t("+ Add Custom Parameter")}</span>
        </button>
      </div>
      <div className="mt-4 max-w-4xl text-center">
        <button className="cursor-pointer text-xs text-primary underline-offset-4 hover:underline" onClick={() => void ops.openSchemaFile()}>
          {t("Open schema file (manual editing for advanced users)")}
        </button>
      </div>

      <AddParamModal
        open={addModalOpen}
        preferredFileId={addParamFileId}
        onClose={() => setAddModalOpen(false)}
      />
    </main>
  );
}
