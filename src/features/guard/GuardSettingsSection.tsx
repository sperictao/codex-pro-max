// 设置-看守分区：总开关 + 看守文件列表 + 添加/编辑文件弹窗（旧 guard.ts 文件管理部分）。
// 归入 guard 特征域，由设置视图作为组合根引用（同 AboutSection 之例）。
// 分区挂载即刷新文件列表（旧 switchSection("guard") 行为）；内置文件无检测记录时自动检测一次。

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { MoreHorizontalIcon } from "lucide-react";
import { toast } from "sonner";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { fmtTs } from "@/shared/lib/format";
import { Modal } from "@/shared/components/Modal";
import { Badge } from "@/shared/components/ui/badge";
import { Button } from "@/shared/components/ui/button";
import { Card, CardTitle } from "@/shared/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/components/ui/dropdown-menu";
import { Input } from "@/shared/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/components/ui/select";
import { Switch } from "@/shared/components/ui/switch";
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
    <Card className="gap-0 rounded-lg p-3 shadow-xs" data-file-id={f.id}>
      <div className="flex items-center gap-2">
        <CardTitle className="text-sm font-medium">{f.name}</CardTitle>
        <Badge variant="outline" className="text-muted-foreground">{f.format}</Badge>
      </div>
      <div className="mt-1 font-mono text-xs text-muted-foreground">~/.codex/{f.file}</div>
      {detText && <div className="mt-1 text-xs tabular-nums text-muted-foreground">{detText}</div>}
      <div className="mt-2 flex gap-2">
        {f.builtin && (
          <Button variant="outline" size="sm" onClick={() => void ops.detectFile(f.id)}>{t("Detect")}</Button>
        )}
        <Button variant="outline" size="sm" onClick={() => onEdit(f.id)}>{t("Edit")}</Button>
        {f.builtin ? (
          <Button variant="outline" size="sm" disabled>{t("Built-in")}</Button>
        ) : (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-sm" aria-label={t("More actions")}><MoreHorizontalIcon /></Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem variant="destructive" onSelect={() => void ops.removeFile(f.id)}>
                {t("Delete")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
    </Card>
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
      toast.error(`${e}`);
    }
  };

  const submit = async () => {
    const ok = await ops.saveFile(editing?.id ?? null, name.trim(), file.trim(), format);
    if (ok) onClose();
  };

  return (
    <Modal
      open={open}
      onRequestClose={onClose}
      labelledBy="guard-file-modal-title"
      title={editing ? t("Edit Guard File") : t("Add Guard File")}
    >
      <h3 className="text-sm font-semibold" id="guard-file-modal-title">
        {editing ? t("Edit Guard File") : t("Add Guard File")}
      </h3>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Name")}</label>
          <Input type="text" placeholder={t("e.g. my-config.toml")}
            value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Format")}</label>
          {/* 编辑模式下格式不可改（后端 guard_update_file 不收 format） */}
          <Select value={format} onValueChange={setFormat} disabled={!!editing}>
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="toml">TOML</SelectItem>
              <SelectItem value="json">JSON</SelectItem>
              <SelectItem value="md">Markdown</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>
      <div>
        <label className="mb-1 block text-xs font-medium">{t("Path (relative to ~/.codex)")}</label>
        <div className="flex gap-2">
          <Input type="text" className="font-mono" placeholder={t("e.g. my-config.toml")}
            value={file} onChange={(e) => setFile(e.target.value)} />
          <Button variant="outline" onClick={() => void pickPath()}>{t("Pick…")}</Button>
        </div>
      </div>
      <div className="mt-1 flex justify-end gap-2">
        <Button variant="outline" onClick={onClose}>{t("Cancel")}</Button>
        <Button onClick={() => void submit()}>{editing ? t("Save") : t("Add")}</Button>
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
        <label className="flex flex-1 cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3" htmlFor="settings-guard-toggle">
          <span className="flex flex-col gap-0.5">
            <span className="text-sm">{t("Enable Codex config guard")}</span>
            <span className="text-xs text-muted-foreground">
              {t("Manage and lock ~/.codex config parameters; locked params are reverted automatically when changed (only while this app is running, every 60 seconds).")}
            </span>
          </span>
          <Switch id="settings-guard-toggle" checked={guardEnabled}
            onCheckedChange={() => void toggleGuardEnabled()} />
        </label>
      </div>

      <div className="flex items-start gap-4 py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("Config Files")}</label>
        <div className="flex-1">
          <div className="mb-2 flex items-center justify-between">
            <span className="text-xs text-muted-foreground">{t("List of guarded files (built-in files cannot be removed)")}</span>
            <Button variant="outline" id="guard-file-form-toggle" onClick={openAdd}>{t("+ Add File")}</Button>
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
