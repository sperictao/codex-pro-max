// 设置-看守分区：总开关 + 看守文件列表 + 添加/编辑文件弹窗（旧 guard.ts 文件管理部分）。
// 归入 guard 特征域，由设置视图作为组合根引用（同 AboutSection 之例）。
// 分区挂载即刷新文件列表（旧 switchSection("guard") 行为）；内置文件无检测记录时自动检测一次。

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { fmtTs } from "@/shared/lib/format";
import { Modal } from "@/shared/components/Modal";
import { BTN, BTN_DANGER_SM, BTN_PRIMARY, BTN_SM, INPUT, INPUT_MONO, SELECT, TOGGLE } from "@/shared/lib/ui";
import type { GuardFileView } from "@/shared/types";
import * as ops from "./ops";

function FileCard({ f, onEdit }: { f: GuardFileView; onEdit: (id: string) => void }) {
  const { t } = useTranslation();
  const det = f.detection;
  const detText = det
    ? det.path === null
      ? t("Detection: file not found ({{at}})", { at: fmtTs(det.at) })
      : det.path === f.file
        ? t("Detection: path matches ({{at}})", { at: fmtTs(det.at) })
        : t("Detection: actually at {{path}} ({{at}})", { path: det.path, at: fmtTs(det.at) })
    : "";
  return (
    <div className="rounded-lg border border-border bg-card text-card-foreground p-3" data-file-id={f.id}>
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium">{f.name}</span>
        <span className="inline-flex items-center rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">{f.format}</span>
      </div>
      <div className="mt-1 font-mono text-xs opacity-60">~/.codex/{f.file}</div>
      {detText && <div className="mt-1 text-xs opacity-60">{detText}</div>}
      <div className="mt-2 flex gap-2">
        {f.builtin && (
          <button className={BTN_SM} onClick={() => void ops.detectFile(f.id)}>{t("Detect")}</button>
        )}
        <button className={BTN_SM} onClick={() => onEdit(f.id)}>{t("Edit")}</button>
        {f.builtin ? (
          <button className={BTN_SM} disabled>{t("Built-in")}</button>
        ) : (
          <button className={BTN_DANGER_SM} onClick={() => void ops.removeFile(f.id)}>{t("Delete")}</button>
        )}
      </div>
    </div>
  );
}

function FileModal({
  open,
  editing,
  onClose,
}: {
  open: boolean;
  editing: GuardFileView | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const toast = useAppStore((s) => s.toast);
  const [name, setName] = useState("");
  const [file, setFile] = useState("");
  const [format, setFormat] = useState("toml");

  // 打开时按添加/编辑定型（旧 toggleGuardFileForm/guardEditFile）
  useEffect(() => {
    if (!open) return;
    setName(editing?.name ?? "");
    setFile(editing?.file ?? "");
    setFormat(editing?.format ?? "toml");
  }, [open, editing]);

  const pickPath = async () => {
    try {
      const selected = await openDialog({ multiple: false });
      if (typeof selected !== "string") return;
      const rel = await cmd.guardRelativizePickedPath(selected);
      setFile(rel);
      // 顺手带入文件名与格式
      const fileName = rel.split("/").pop() ?? rel;
      if (!name.trim()) setName(fileName);
      const ext = fileName.split(".").pop()?.toLowerCase();
      if (ext === "toml" || ext === "json" || ext === "md") setFormat(ext);
    } catch (e) {
      toast(`${e}`, "error");
    }
  };

  const submit = async () => {
    const ok = await ops.saveFile(editing?.id ?? null, name.trim(), file.trim(), format);
    if (ok) onClose();
  };

  return (
    <Modal open={open} onOverlayClick={onClose} labelledBy="guard-file-modal-title">
      <h3 className="text-sm font-semibold" id="guard-file-modal-title">
        {editing ? t("Edit Guard File") : t("Add Guard File")}
      </h3>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Name")}</label>
          <input type="text" className={INPUT} placeholder={t("e.g. my-config.toml")}
            value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Format")}</label>
          {/* 编辑模式下格式不可改（后端 guard_update_file 不收 format） */}
          <select className={SELECT} value={format} disabled={!!editing} onChange={(e) => setFormat(e.target.value)}>
            <option value="toml">TOML</option>
            <option value="json">JSON</option>
            <option value="md">Markdown</option>
          </select>
        </div>
      </div>
      <div>
        <label className="mb-1 block text-xs font-medium">{t("Path (relative to ~/.codex)")}</label>
        <div className="flex gap-2">
          <input type="text" className={INPUT_MONO} placeholder={t("e.g. my-config.toml")}
            value={file} onChange={(e) => setFile(e.target.value)} />
          <button className={BTN} onClick={() => void pickPath()}>{t("Pick…")}</button>
        </div>
      </div>
      <div className="mt-1 flex justify-end gap-2">
        <button className={BTN} onClick={onClose}>{t("Cancel")}</button>
        <button className={BTN_PRIMARY} onClick={() => void submit()}>{editing ? t("Save") : t("Add")}</button>
      </div>
    </Modal>
  );
}

export function GuardSettingsSection() {
  const { t } = useTranslation();
  const guardEnabled = useAppStore((s) => s.guardState.enabled);
  const toggleGuardEnabled = useAppStore((s) => s.toggleGuardEnabled);
  const guardFiles = useAppStore((s) => s.guardFiles);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<GuardFileView | null>(null);

  useEffect(() => {
    void ops.refreshFiles();
  }, []);

  const openAdd = () => {
    setEditing(null);
    setModalOpen(true);
  };
  const openEdit = (id: string) => {
    const f = guardFiles.find((x) => x.id === id);
    if (!f) return;
    setEditing(f);
    setModalOpen(true);
  };

  return (
    <section className="settings-section" id="section-guard">
      <h2 className="mb-4 text-base font-semibold">{t("Config Guard")}</h2>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("Master Switch")}</label>
        <label className="flex flex-1 cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3">
          <span className="flex flex-col gap-0.5">
            <span className="text-sm">{t("Enable Codex config guard")}</span>
            <span className="text-xs opacity-60">
              {t("Manage and lock ~/.codex config parameters; locked params are reverted automatically when changed (only while this app is running, every 60 seconds).")}
            </span>
          </span>
          <input type="checkbox" className={TOGGLE} id="settings-guard-toggle"
            checked={guardEnabled} onChange={() => void toggleGuardEnabled()} />
        </label>
      </div>

      <div className="flex items-start gap-4 py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("Config Files")}</label>
        <div className="flex-1">
          <div className="mb-2 flex items-center justify-between">
            <span className="text-xs opacity-60">{t("List of guarded files (built-in files cannot be removed)")}</span>
            <button className={BTN} id="guard-file-form-toggle" onClick={openAdd}>{t("+ Add File")}</button>
          </div>
          <div className="flex flex-col gap-2" id="settings-guard-files">
            {guardFiles.length === 0
              ? t("No files yet")
              : guardFiles.map((f) => <FileCard key={f.id} f={f} onEdit={openEdit} />)}
          </div>
        </div>
      </div>

      <FileModal open={modalOpen} editing={editing} onClose={() => setModalOpen(false)} />
    </section>
  );
}
