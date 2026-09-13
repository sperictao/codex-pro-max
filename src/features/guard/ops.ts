// guard/ops：看守域全部操作（旧 guard.ts 的逻辑层）。数据落 store，组件只调这里。
// 参数操作前先拉取最新视图定位参数（与旧实现一致，不用可能过期的快照做取反/判断）。

import { ask } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useAppStore } from "@/shared/store";
import { toast } from "sonner";
import * as cmd from "@/shared/commands";
import { log } from "@/shared/logger";
import { i18n } from "@/shared/i18n";
import type { CustomParamPayload, GuardParamView, GuardView } from "@/shared/types";

const store = () => useAppStore.getState();
const t = (key: string, params?: Record<string, string | number>) => i18n.t(key, params);

async function findParam(id: string): Promise<(GuardParamView & { file: string }) | null> {
  const view: GuardView = await cmd.guardGetView();
  for (const g of view.groups) {
    const p = g.params.find((x) => x.id === id);
    if (p) return { ...p, file: g.file };
  }
  return null;
}

// ============ 参数操作 ============

export async function toggleBool(id: string, next: boolean): Promise<void> {
  try {
    const p = await findParam(id);
    if (!p) return;
    if (p.locked) return;
    await cmd.guardSetValue(id, next);
    await store().refreshGuardView(true);
  } catch (e) {
    toast.error(t("Change failed: {{error}}", { error: String(e) }));
  }
}

export async function setValue(id: string, raw: string): Promise<void> {
  try {
    const p = await findParam(id);
    if (!p) return;
    const value = p.valueType === "int" ? parseInt(raw, 10) : raw;
    if (p.valueType === "int" && Number.isNaN(value)) {
      toast.error(t("Please enter an integer"));
      await store().refreshGuardView(true);
      return;
    }
    await cmd.guardSetValue(id, value);
    await store().refreshGuardView(true);
  } catch (e) {
    toast.error(t("Save failed: {{error}}", { error: String(e) }));
    await store().refreshGuardView(true);
  }
}

export async function applyParam(id: string): Promise<void> {
  try {
    await cmd.guardApply(id);
    toast.success(t("Applied"));
  } catch (e) {
    toast.error(t("Apply failed: {{error}}", { error: String(e) }));
  }
  await store().refreshGuardView(true);
}

export async function disableParam(id: string): Promise<void> {
  try {
    await cmd.guardSetApplied(id, false);
    toast.info(t("Disabled"));
  } catch (e) {
    toast.error(t("Operation failed: {{error}}", { error: String(e) }));
  }
  await store().refreshGuardView(true);
}

export async function toggleApplied(id: string): Promise<void> {
  try {
    const p = await findParam(id);
    if (!p) return;
    if (p.applied) {
      await disableParam(id);
    } else {
      await applyParam(id);
    }
  } catch (e) {
    toast.error(t("Operation failed: {{error}}", { error: String(e) }));
    await store().refreshGuardView(true);
  }
}

export async function setLocked(id: string, locked: boolean): Promise<void> {
  try {
    await cmd.guardSetLocked(id, locked);
    if (locked) toast.success(t("Locked"));
    else toast.info(t("Unlocked"));
  } catch (e) {
    toast.error(t("Operation failed: {{error}}", { error: String(e) }));
  }
  await store().refreshGuardView(true);
}

export async function removeConfig(id: string): Promise<void> {
  const p = await findParam(id);
  if (!p) return;
  const label = p.label || p.id;
  const ok = await ask(
    t("Remove config for {{label}}?\n\nThis will delete the parameter's value from ~/.codex/{{file}} (a backup is saved to ~/.codex/dashi-backups/). The guard entry itself stays; only the written config is removed.", { label, file: p.file }),
    { title: t("Remove Config"), kind: "warning" },
  );
  if (!ok) return;
  try {
    await cmd.guardRemoveConfig(id);
    toast.success(t("Config removed"));
    await store().refreshGuardView(true);
  } catch (e) {
    toast.error(t("Operation failed: {{error}}", { error: String(e) }));
  }
}

export async function removeCustom(id: string): Promise<void> {
  const ok = await ask(
    t("Delete custom parameter {{id}}?\n\nGuarding stops after deletion. Values already written to ~/.codex/ will not be rolled back; restore manually from ~/.codex/dashi-backups/ if needed.", { id }),
    { title: t("Delete Custom Parameter"), kind: "warning" },
  );
  if (!ok) return;
  try {
    await cmd.guardRemoveCustomParam(id);
    toast.success(t("Deleted"));
    await store().refreshGuardView(true);
  } catch (e) {
    toast.error(t("Delete failed: {{error}}", { error: String(e) }));
  }
}

export interface AddCustomForm {
  id: string;
  label: string;
  fileId: string;
  mode: string;
  path: string;
  valueType: string;
  desc: string;
  defaultRaw: string;
}

function parseDefaultValue(value: string, effectiveType: string): unknown {
  switch (effectiveType) {
    case "bool":
      return value === "true";
    case "int": {
      const n = parseInt(value, 10);
      if (Number.isNaN(n)) throw new Error(t("Default value must be an integer"));
      return n;
    }
    case "string":
    case "text":
      return value;
    case "none":
      return null;
    default:
      return value;
  }
}

// 返回 true 表示添加成功（调用方清空表单并关弹窗）
export async function addCustom(form: AddCustomForm): Promise<boolean> {
  if (!form.id) { toast.error(t("Please enter an ID")); return false; }
  if (!form.label) { toast.error(t("Please enter a name")); return false; }
  if (!form.fileId) { toast.error(t("Please select a target file")); return false; }
  if ((form.mode === "toml_key" || form.mode === "toml_absent") && !form.path) {
    toast.error(t("Please enter a TOML path"));
    return false;
  }
  try {
    const effectiveType = form.mode === "file_overwrite" || form.mode === "markdown_block" ? "text" : form.valueType;
    const param: CustomParamPayload = {
      id: form.id,
      label: form.label,
      description: form.desc,
      file: "",
      apply_mode: form.mode,
      path: form.path,
      value_type: effectiveType,
      default: parseDefaultValue(form.defaultRaw, effectiveType),
      custom: true,
    };
    await cmd.guardAddCustomParam(param, form.fileId);
    toast.success(t("Custom parameter added"));
    await store().refreshGuardView(true);
    return true;
  } catch (e) {
    toast.error(t("Add failed: {{error}}", { error: String(e) }));
    return false;
  }
}

export async function openSchemaFile(): Promise<void> {
  try {
    await openUrl(await cmd.guardGetSchemaFilePath());
  } catch (e) {
    // 回退：复制路径到剪贴板
    try {
      const path = await cmd.guardGetSchemaFilePath();
      await navigator.clipboard.writeText(path);
      toast.info(t("Path copied to clipboard: {{path}}", { path }));
    } catch {
      toast.error(t("Open failed: {{error}}", { error: String(e) }));
    }
  }
}

// ============ 文件管理 ============

// 拉取文件列表；内置且无检测记录的文件自动检测一次并落盘（旧 refreshGuardFiles 行为）
export async function refreshFiles(): Promise<void> {
  try {
    const files = await cmd.guardGetFiles();
    store().setGuardFiles(files);
    for (const f of files) {
      if (f.builtin && !f.detection) {
        await detectFile(f.id, true);
      }
    }
  } catch (e) {
    log.error("加载看守文件列表", e);
  }
}

export async function detectFile(id: string, auto = false): Promise<void> {
  const f = store().guardFiles.find((x) => x.id === id);
  if (!f) return;
  try {
    const updated = await cmd.guardDetectFile(id);
    store().setGuardFiles(store().guardFiles.map((x) => (x.id === id ? updated : x)));
    const detected = updated.detection?.path ?? null;
    if (detected && detected !== updated.file) {
      const ok = await ask(
        t("\"{{name}}\" was detected at:\n~/.codex/{{detected}}\n\nIt differs from the configured ~/.codex/{{file}}. Update to the detected path?", {
          name: updated.name, detected, file: updated.file,
        }),
        { title: t("Update Guard Path"), kind: "warning" },
      );
      if (ok) {
        await cmd.guardUpdateFile(id, updated.name, detected);
        toast.success(t("Updated to the detected path"));
        await refreshFiles();
        await store().refreshGuardView(true);
      }
    } else if (!auto) {
      if (detected) toast.success(t("Detection complete: path matches"));
      else toast.info(t("File not found under ~/.codex"));
    }
  } catch (e) {
    if (!auto) toast.error(t("Detection failed: {{error}}", { error: String(e) }));
  }
}

export async function saveFile(editingId: string | null, name: string, file: string, format: string): Promise<boolean> {
  if (!name) { toast.error(t("Please enter a file name")); return false; }
  if (!file) { toast.error(t("Please enter a file path")); return false; }
  try {
    if (editingId) {
      await cmd.guardUpdateFile(editingId, name, file);
      toast.success(t("Updated"));
    } else {
      await cmd.guardAddFile(name, file, format);
      toast.success(t("File added"));
    }
    await refreshFiles();
    await store().refreshGuardView(true);
    return true;
  } catch (e) {
    toast.error(t(editingId ? "Update failed: {{error}}" : "Add failed: {{error}}", { error: String(e) }));
    return false;
  }
}

export async function removeFile(id: string): Promise<void> {
  const f = store().guardFiles.find((x) => x.id === id);
  if (!f) return;
  const ok = await ask(
    t("Delete file \"{{name}}\"?\n\nAll custom parameters under it will be unguarded, but values already written to ~/.codex/{{file}} will not be rolled back.", { name: f.name, file: f.file }),
    { title: t("Delete Guard File"), kind: "warning" },
  );
  if (!ok) return;
  try {
    await cmd.guardRemoveFile(id);
    toast.success(t("Deleted"));
    await refreshFiles();
    await store().refreshGuardView(true);
  } catch (e) {
    toast.error(t("Delete failed: {{error}}", { error: String(e) }));
  }
}
