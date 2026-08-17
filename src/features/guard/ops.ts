// guard/ops：看守域全部操作（旧 guard.ts 的逻辑层）。数据落 store，组件只调这里。
// 参数操作前先拉取最新视图定位参数（与旧实现一致，不用可能过期的快照做取反/判断）。

import { ask } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { log } from "@/shared/logger";
import { i18n } from "@/shared/i18n";
import type { CustomParamPayload, GuardParamView, GuardView } from "@/shared/types";

const store = () => useAppStore.getState();
const t = (key: string, params?: Record<string, string | number>) => i18n.t(key, params);

async function findParam(id: string): Promise<GuardParamView | null> {
  const view: GuardView = await cmd.guardGetView();
  return view.groups.flatMap((g) => g.params).find((x) => x.id === id) ?? null;
}

// ============ 参数操作 ============

export async function toggleBool(id: string): Promise<void> {
  if (store().guardState.params[id]?.locked) return;
  try {
    const p = await findParam(id);
    if (!p) return;
    await cmd.guardSetValue(id, p.value !== true);
    await store().refreshGuardView(true);
  } catch (e) {
    store().toast(t("Change failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function setValue(id: string, raw: string): Promise<void> {
  try {
    const p = await findParam(id);
    if (!p) return;
    const value = p.valueType === "int" ? parseInt(raw, 10) : raw;
    if (p.valueType === "int" && Number.isNaN(value)) {
      store().toast(t("Please enter an integer"), "error");
      await store().refreshGuardView(true);
      return;
    }
    await cmd.guardSetValue(id, value);
    await store().refreshGuardView(true);
  } catch (e) {
    store().toast(t("Save failed: {{error}}", { error: String(e) }), "error");
    await store().refreshGuardView(true);
  }
}

export async function applyParam(id: string): Promise<void> {
  try {
    await cmd.guardApply(id);
    store().toast(t("Applied"), "success");
  } catch (e) {
    store().toast(t("Apply failed: {{error}}", { error: String(e) }), "error");
  }
  await store().refreshGuardView(true);
}

export async function disableParam(id: string): Promise<void> {
  try {
    await cmd.guardSetApplied(id, false);
    store().toast(t("Disabled"), "info");
  } catch (e) {
    store().toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
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
    store().toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
    await store().refreshGuardView(true);
  }
}

export async function setLocked(id: string, locked: boolean): Promise<void> {
  try {
    await cmd.guardSetLocked(id, locked);
    store().toast(locked ? t("Locked") : t("Unlocked"), locked ? "success" : "info");
  } catch (e) {
    store().toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
  }
  await store().refreshGuardView(true);
}

export async function removeCustom(id: string): Promise<void> {
  const ok = await ask(
    t("Delete custom parameter {{id}}?\n\nGuarding stops after deletion. Values already written to ~/.codex/ will not be rolled back; restore manually from ~/.codex/dashi-backups/ if needed.", { id }),
    { title: t("Delete Custom Parameter"), kind: "warning" },
  );
  if (!ok) return;
  try {
    await cmd.guardRemoveCustomParam(id);
    store().toast(t("Deleted"), "success");
    await store().refreshGuardView(true);
  } catch (e) {
    store().toast(t("Delete failed: {{error}}", { error: String(e) }), "error");
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
  if (!form.id) { store().toast(t("Please enter an ID"), "error"); return false; }
  if (!form.label) { store().toast(t("Please enter a name"), "error"); return false; }
  if (!form.fileId) { store().toast(t("Please select a target file"), "error"); return false; }
  if ((form.mode === "toml_key" || form.mode === "toml_absent") && !form.path) {
    store().toast(t("Please enter a TOML path"), "error");
    return false;
  }
  try {
    const effectiveType = form.mode === "file_overwrite" || form.mode === "markdown_block" ? "text" : form.valueType;
    const param: CustomParamPayload = {
      id: form.id,
      label: form.label,
      description: form.desc,
      file: "",
      applyMode: form.mode,
      path: form.path,
      valueType: effectiveType,
      default: parseDefaultValue(form.defaultRaw, effectiveType),
      custom: true,
    };
    await cmd.guardAddCustomParam(param, form.fileId);
    store().toast(t("Custom parameter added"), "success");
    await store().refreshGuardView(true);
    return true;
  } catch (e) {
    store().toast(t("Add failed: {{error}}", { error: String(e) }), "error");
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
      store().toast(t("Path copied to clipboard: {{path}}", { path }), "info");
    } catch {
      store().toast(t("Open failed: {{error}}", { error: String(e) }), "error");
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
        store().toast(t("Updated to the detected path"), "success");
        await refreshFiles();
        await store().refreshGuardView(true);
      }
    } else if (!auto) {
      store().toast(
        detected ? t("Detection complete: path matches") : t("File not found under ~/.codex"),
        detected ? "success" : "info",
      );
    }
  } catch (e) {
    if (!auto) store().toast(t("Detection failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function saveFile(editingId: string | null, name: string, file: string, format: string): Promise<boolean> {
  if (!name) { store().toast(t("Please enter a file name"), "error"); return false; }
  if (!file) { store().toast(t("Please enter a file path"), "error"); return false; }
  try {
    if (editingId) {
      await cmd.guardUpdateFile(editingId, name, file);
      store().toast(t("Updated"), "success");
    } else {
      await cmd.guardAddFile(name, file, format);
      store().toast(t("File added"), "success");
    }
    await refreshFiles();
    await store().refreshGuardView(true);
    return true;
  } catch (e) {
    store().toast(t(editingId ? "Update failed: {{error}}" : "Add failed: {{error}}", { error: String(e) }), "error");
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
    store().toast(t("Deleted"), "success");
    await refreshFiles();
    await store().refreshGuardView(true);
  } catch (e) {
    store().toast(t("Delete failed: {{error}}", { error: String(e) }), "error");
  }
}
