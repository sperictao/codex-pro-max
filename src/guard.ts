// guard：看守视图渲染与参数操作、看守文件管理、自定义参数管理

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, ask } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { t } from "./i18n";
import { toast, escapeHtml, fmtTs } from "./core";
import { state } from "./state";
import { showHome } from "./nav";

interface GuardParamView {
  id: string;
  label: string;
  description: string;
  applyMode: string;
  valueType: string;
  path: string;
  default: unknown;
  value: unknown;
  applied: boolean;
  locked: boolean;
  actual: string | null;
  status: "match" | "drift" | "missing" | "error";
  error: string | null;
  lastChecked: number | null;
  lastRestored: number | null;
  custom: boolean;
}

interface GuardGroupView {
  id: string;
  name: string;
  file: string;
  format: string;
  builtin: boolean;
  error: string | null;
  params: GuardParamView[];
}

interface GuardFileView {
  id: string;
  name: string;
  file: string;
  format: string;
  builtin: boolean;
  detection: { path: string | null; at: number } | null;
}

interface GuardView {
  enabled: boolean;
  groups: GuardGroupView[];
}

// ============ 总开关 ============
export async function toggleGuard(): Promise<void> {
  const enabled = !state.guardState.enabled;
  try {
    await invoke("guard_set_enabled", { enabled });
    state.guardState.enabled = enabled;
    renderGuardToggle();
    toast(enabled ? t("Config guard enabled") : t("Config guard disabled"), enabled ? "success" : "info");
  } catch (e) {
    renderGuardToggle();
    toast(t("Toggle failed: {{error}}", { error: String(e) }), "error");
  }
}

export function renderGuardToggle(): void {
  const el = document.getElementById("settings-guard-toggle") as HTMLInputElement | null;
  if (el) el.checked = state.guardState.enabled;
  // 总开关关闭时隐藏顶部「看守」Tab
  const btn = document.getElementById("btn-guard");
  if (btn) btn.classList.toggle("hidden", !state.guardState.enabled);
  // 如果关了总开关且当前在看守页，跳回主页
  if (!state.guardState.enabled) {
    const view = document.getElementById("guard-view");
    if (view && !view.classList.contains("hidden")) {
      showHome();
    }
  }
}

// ============ 看守视图 ============
let lastGuardJson = "";

export async function refreshGuardView(force = false): Promise<void> {
  const viewEl = document.getElementById("guard-view");
  if (!viewEl || viewEl.classList.contains("hidden")) return;
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const json = JSON.stringify(view);
    if (!force && json === lastGuardJson) return;
    // 用户正在输入时不重渲染，避免抢走焦点/清空草稿
    const ae = document.activeElement;
    if (!force && ae && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA")
        && viewEl.contains(ae)) return;
    lastGuardJson = json;
    renderGuardView(view);
    renderGuardToggle();
  } catch {
    // 轮询错误忽略
  }
}

const LOCK_SVG = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>`;
const UNLOCK_SVG = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 9.9-1"/></svg>`;

function renderGuardView(view: GuardView): void {
  const container = document.getElementById("guard-groups")!;
  const statusMap: Record<string, { text: string; cls: string }> = {
    match: { text: t("Match"), cls: "running" },
    drift: { text: t("Drift"), cls: "failed" },
    missing: { text: t("Missing"), cls: "starting" },
    error: { text: t("Error"), cls: "failed" },
  };
  // 渲染内容来自本地 schema/配置文件，非远程输入；动态文本一律 escapeHtml
  container.innerHTML = view.groups.map((g) => {
    const params = g.params.map((p) => {
      const s = statusMap[p.status] ?? statusMap.error;
      let editor = "";
      const dis = p.locked ? "disabled" : "";
      if (p.valueType === "bool") {
        editor = `<div class="flex items-center gap-2">
          <input type="checkbox" class="relative h-5 w-9 shrink-0 cursor-pointer appearance-none rounded-full bg-input transition-colors outline-none checked:bg-primary focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 before:absolute before:left-0.5 before:top-0.5 before:h-4 before:w-4 before:rounded-full before:bg-background before:shadow-sm before:transition-transform checked:before:translate-x-4" data-change-action="guardToggleBool" data-id="${p.id}"
                 ${p.value === true ? "checked" : ""} ${p.locked ? "disabled" : ""} />
          <span class="text-xs opacity-70">${p.value === true ? "true" : "false"} ${t("(recommended {{default}})", { default: String(p.default) })}</span>
        </div>`;
      } else if (p.valueType === "int" || p.valueType === "string") {
        const t = p.valueType === "int" ? "number" : "text";
        editor = `<input type="${t}" class="h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-xs outline-none transition-colors placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 font-mono" ${dis}
               value="${escapeHtml(String(p.value ?? ""))}" data-guard-id="${p.id}"
               data-change-action="guardSetValue" data-id="${p.id}" />`;
      } else if (p.valueType === "text") {
        editor = `<textarea class="guard-textarea" ${dis} data-guard-id="${p.id}"
               data-change-action="guardSetValue" data-id="${p.id}">${escapeHtml(String(p.value ?? ""))}</textarea>`;
      } else {
        editor = `<span class="text-xs opacity-60">${t("No editable value; applying performs \"{{action}}\"", { action: t(p.applyMode === "toml_absent" ? "delete" : "write") })}</span>`;
      }
      const meta = p.locked
        ? `<div class="mt-1 text-xs opacity-50">${t("Last checked {{checked}} | Last auto-restored {{restored}}", { checked: fmtTs(p.lastChecked), restored: fmtTs(p.lastRestored) })}</div>`
        : "";
      return `<div class="guard-param-card rounded-lg border border-border bg-card text-card-foreground p-3">
        <div class="flex flex-wrap items-center gap-2">
          <span class="text-sm font-medium">${escapeHtml(p.label)}${p.description || p.path ? ` <span class="guard-param-help" tabindex="0">?<span class="guard-param-desc">${p.description ? `<span>${escapeHtml(p.description)}</span>` : ""}${p.path ? `<span class="guard-param-desc-path">${escapeHtml(p.path)}</span>` : ""}</span></span>` : ""}</span>
          <span class="guard-param-actual ${p.status === "match" ? "ok" : "bad"}">${t("Current: ")}${escapeHtml(p.actual ?? p.error ?? t("Unknown"))}</span>
          <span class="status-badge ${s.cls}"><span class="dot"></span><span>${s.text}</span></span>
        </div>
        <div class="mt-1 flex items-start justify-between gap-2">
          <div class="min-w-0 flex-1">
            <div class="mt-2">${editor}</div>
            ${meta}
          </div>
          <span class="guard-param-actions flex w-[30%] shrink-0 flex-row flex-wrap items-center justify-end gap-1 self-center">
            <input type="checkbox" class="text-switch" data-change-action="guardToggleApplied" data-id="${p.id}" data-state-text="${p.applied ? t("Enabled") : t("Disabled")}"
                   ${p.applied ? "checked" : ""} ${p.locked ? "disabled" : ""} title="${p.applied ? t("Disable") : t("Enable")}" aria-label="${p.applied ? t("Disable") : t("Enable")}" />
            ${p.locked
              ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-1 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="guardSetLocked" data-id="${p.id}" data-locked="false">${LOCK_SVG}${t("Unlock")}</button>`
              : `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-1 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" ${p.applied ? "" : "disabled"}
                    data-action="guardSetLocked" data-id="${p.id}" data-locked="true">${UNLOCK_SVG}${t("Lock")}</button>`}
            ${p.custom ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-destructive/50 bg-background text-destructive hover:bg-destructive/10 h-6 px-2 text-xs" data-action="guardRemoveCustom" data-id="${p.id}" title="${t("Delete custom parameter")}">${t("Delete")}</button>` : ""}
          </span>
        </div>
      </div>`;
    }).join("");
    const addBtn = `<div class="mt-2">
      <button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="openGuardAddFormFor" data-id="${g.id}">${t("+ Add Parameter")}</button>
    </div>`;
    return `<div class="rounded-xl border border-border bg-card text-card-foreground p-4" data-group-id="${g.id}">
      <div class="text-sm font-semibold">${escapeHtml(g.name)}</div>
      <div class="mb-2 font-mono text-xs opacity-50">~/.codex/${escapeHtml(g.file)}</div>
      ${g.error ? `<div class="mb-2 text-xs text-destructive">${escapeHtml(g.error)}</div>` : ""}
      <div class="flex flex-col gap-2">${params}</div>
      ${addBtn}
    </div>`;
  }).join("");
}

export async function guardToggleBool(id: string): Promise<void> {
  const st = state.guardState.params[id];
  if (st?.locked) return;
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    await invoke("guard_set_value", { id, value: p.value !== true });
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Change failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function guardSetValue(id: string, input: HTMLInputElement | HTMLTextAreaElement): Promise<void> {
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    const value = p.valueType === "int" ? parseInt(input.value, 10) : input.value;
    if (p.valueType === "int" && Number.isNaN(value)) {
      toast(t("Please enter an integer"), "error");
      await refreshGuardView(true);
      return;
    }
    await invoke("guard_set_value", { id, value });
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Save failed: {{error}}", { error: String(e) }), "error");
    await refreshGuardView(true);
  }
}

export async function guardApply(id: string): Promise<void> {
  try {
    await invoke("guard_apply", { id });
    toast(t("Applied"), "success");
  } catch (e) {
    toast(t("Apply failed: {{error}}", { error: String(e) }), "error");
  }
  await refreshGuardView(true);
}

export async function guardToggleApplied(id: string): Promise<void> {
  try {
    const view = await invoke<GuardView>("guard_get_view");
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    if (p.applied) {
      await guardDisable(id);
    } else {
      await guardApply(id);
    }
  } catch (e) {
    toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
    await refreshGuardView(true);
  }
}

export async function guardDisable(id: string): Promise<void> {
  try {
    await invoke("guard_set_applied", { id, applied: false });
    toast(t("Disabled"), "info");
  } catch (e) {
    toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
  }
  await refreshGuardView(true);
}

export async function guardSetLocked(id: string, locked: boolean): Promise<void> {
  try {
    await invoke("guard_set_locked", { id, locked });
    toast(locked ? t("Locked") : t("Unlocked"), locked ? "success" : "info");
  } catch (e) {
    toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
  }
  await refreshGuardView(true);
}

// ============ 文件管理 ============
let guardFiles: GuardFileView[] = [];
let guardAddParamFileId: string | null = null;

export async function refreshGuardFiles(): Promise<void> {
  try {
    guardFiles = await invoke<GuardFileView[]>("guard_get_files");
    renderGuardFiles();
    // 首次（无检测记录）自动检测一次并落盘；之后直接读记录，不重复扫盘
    for (const f of guardFiles) {
      if (f.builtin && !f.detection) {
        await guardDetectFile(f.id, true);
      }
    }
  } catch (e) {
    console.error("加载文件列表失败", e);
  }
}

function renderGuardFileSelect(): void {
  const sel = document.getElementById("guard-add-file-select") as HTMLSelectElement | null;
  if (!sel) return;
  const prev = sel.value;
  sel.innerHTML = guardFiles.map((f) =>
    `<option value="${f.id}">${escapeHtml(f.name)} (${f.format})</option>`
  ).join("");
  if (guardAddParamFileId && guardFiles.some((f) => f.id === guardAddParamFileId)) {
    sel.value = guardAddParamFileId;
  } else if (prev && guardFiles.some((f) => f.id === prev)) {
    sel.value = prev;
  }
}

export function renderGuardFiles(): void {
  // 设置页里的文件列表
  const container = document.getElementById("settings-guard-files");
  if (container) {
    if (guardFiles.length === 0) {
      container.textContent = t("No files yet");
    } else {
      // 渲染内容来自本地配置文件，动态文本一律 escapeHtml
      const html = guardFiles.map((f) => {
        const delBtn = f.builtin
          ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" disabled>${t("Built-in")}</button>`
          : `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-destructive/50 bg-background text-destructive hover:bg-destructive/10 h-6 px-2 text-xs" data-action="guardRemoveFile" data-id="${f.id}">${t("Delete")}</button>`;
        const detectBtn = f.builtin
          ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="guardDetectFile" data-id="${f.id}">${t("Detect")}</button>`
          : "";
        const det = f.detection;
        const detText = det
          ? det.path === null
            ? t("Detection: file not found ({{at}})", { at: fmtTs(det.at) })
            : det.path === f.file
              ? t("Detection: path matches ({{at}})", { at: fmtTs(det.at) })
              : t("Detection: actually at {{path}} ({{at}})", { path: det.path, at: fmtTs(det.at) })
          : "";
        return `<div class="rounded-lg border border-border bg-card text-card-foreground p-3" data-file-id="${f.id}">
          <div class="flex items-center gap-2">
            <span class="text-sm font-medium">${escapeHtml(f.name)}</span>
            <span class="inline-flex items-center rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">${f.format}</span>
          </div>
          <div class="mt-1 font-mono text-xs opacity-60">~/.codex/${escapeHtml(f.file)}</div>
          ${detText ? `<div class="mt-1 text-xs opacity-60">${escapeHtml(detText)}</div>` : ""}
          <div class="mt-2 flex gap-2">
            ${detectBtn}
            <button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="guardEditFile" data-id="${f.id}">${t("Edit")}</button>
            ${delBtn}
          </div>
        </div>`;
      }).join("");
      container.innerHTML = html;
    }
  }
  // 添加参数表单里的文件下拉
  renderGuardFileSelect();
}

let editingFileId: string | null = null;

export function toggleGuardFileForm(): void {
  const modal = document.getElementById("guard-file-modal");
  if (modal) modal.classList.toggle("hidden");
  editingFileId = null;
  const submit = document.getElementById("settings-guard-file-submit");
  if (submit) submit.textContent = t("Add");
  const title = document.getElementById("guard-file-modal-title");
  if (title) title.textContent = t("Add Guard File");
  // 编辑模式下格式不可改（后端 guard_update_file 不收 format），添加时恢复可选
  const formatSel = document.getElementById("settings-guard-file-format") as HTMLSelectElement | null;
  if (formatSel) formatSel.disabled = false;
}

export function guardEditFile(id: string): void {
  const f = guardFiles.find((x) => x.id === id);
  if (!f) return;
  editingFileId = id;
  (document.getElementById("settings-guard-file-name") as HTMLInputElement).value = f.name;
  (document.getElementById("settings-guard-file-path") as HTMLInputElement).value = f.file;
  const formatSel = document.getElementById("settings-guard-file-format") as HTMLSelectElement;
  formatSel.value = f.format;
  formatSel.disabled = true;
  document.getElementById("settings-guard-file-submit")!.textContent = t("Save");
  document.getElementById("guard-file-modal-title")!.textContent = t("Edit Guard File");
  document.getElementById("guard-file-modal")!.classList.remove("hidden");
}

export async function guardPickFilePath(): Promise<void> {
  try {
    const selected = await openDialog({ multiple: false });
    if (typeof selected !== "string") return;
    const rel = await invoke<string>("guard_relativize_picked_path", { absPath: selected });
    (document.getElementById("settings-guard-file-path") as HTMLInputElement).value = rel;
    // 顺手带入文件名与格式
    const nameEl = document.getElementById("settings-guard-file-name") as HTMLInputElement;
    const fileName = rel.split("/").pop() ?? rel;
    if (!nameEl.value.trim()) nameEl.value = fileName;
    const ext = fileName.split(".").pop()?.toLowerCase();
    if (ext === "toml" || ext === "json" || ext === "md") {
      (document.getElementById("settings-guard-file-format") as HTMLSelectElement).value = ext;
    }
  } catch (e) {
    toast(`${e}`, "error");
  }
}

export async function guardSaveFileForm(): Promise<void> {
  const name = (document.getElementById("settings-guard-file-name") as HTMLInputElement).value.trim();
  const file = (document.getElementById("settings-guard-file-path") as HTMLInputElement).value.trim();
  const format = (document.getElementById("settings-guard-file-format") as HTMLSelectElement).value;
  if (!name) { toast(t("Please enter a file name"), "error"); return; }
  if (!file) { toast(t("Please enter a file path"), "error"); return; }
  try {
    if (editingFileId) {
      await invoke("guard_update_file", { id: editingFileId, name, file });
      toast(t("Updated"), "success");
    } else {
      await invoke("guard_add_file", { name, file, format });
      toast(t("File added"), "success");
    }
    (document.getElementById("settings-guard-file-name") as HTMLInputElement).value = "";
    (document.getElementById("settings-guard-file-path") as HTMLInputElement).value = "";
    toggleGuardFileForm();
    await refreshGuardFiles();
    await refreshGuardView(true);
  } catch (e) {
    toast(t(editingFileId ? "Update failed: {{error}}" : "Add failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function guardDetectFile(id: string, auto = false): Promise<void> {
  const f = guardFiles.find((x) => x.id === id);
  if (!f) return;
  try {
    const updated = await invoke<GuardFileView>("guard_detect_file", { id });
    guardFiles = guardFiles.map((x) => (x.id === id ? updated : x));
    renderGuardFiles();
    const detected = updated.detection?.path ?? null;
    if (detected && detected !== updated.file) {
      const ok = await ask(
        t("\"{{name}}\" was detected at:\n~/.codex/{{detected}}\n\nIt differs from the configured ~/.codex/{{file}}. Update to the detected path?", {
          name: updated.name, detected, file: updated.file,
        }),
        { title: t("Update Guard Path"), kind: "warning" }
      );
      if (ok) {
        await invoke("guard_update_file", { id, name: updated.name, file: detected });
        toast(t("Updated to the detected path"), "success");
        await refreshGuardFiles();
        await refreshGuardView(true);
      }
    } else if (!auto) {
      toast(detected ? t("Detection complete: path matches") : t("File not found under ~/.codex"), detected ? "success" : "info");
    }
  } catch (e) {
    if (!auto) toast(t("Detection failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function guardRemoveFile(id: string): Promise<void> {
  const f = guardFiles.find((x) => x.id === id);
  if (!f) return;
  if (!(await ask(t("Delete file \"{{name}}\"?\n\nAll custom parameters under it will be unguarded, but values already written to ~/.codex/{{file}} will not be rolled back.", { name: f.name, file: f.file }), { title: t("Delete Guard File"), kind: "warning" }))) {
    return;
  }
  try {
    await invoke("guard_remove_file", { id });
    toast(t("Deleted"), "success");
    await refreshGuardFiles();
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Delete failed: {{error}}", { error: String(e) }), "error");
  }
}

// ============ 自定义参数管理 ============
export function openGuardAddFormFor(fileId: string): void {
  guardAddParamFileId = fileId;
  const fileSelect = document.getElementById("guard-add-file-select") as HTMLSelectElement | null;
  if (fileSelect) {
    fileSelect.value = fileId;
  }
  openGuardAddModal();
}

export function openGuardAddModal(): void {
  document.getElementById("guard-add-modal")!.classList.remove("hidden");
  onGuardAddModeChange();
  onGuardAddValueTypeChange();
}

export function closeGuardAddModal(): void {
  document.getElementById("guard-add-modal")!.classList.add("hidden");
}

export function onGuardAddModeChange(): void {
  const mode = (document.getElementById("guard-add-mode") as HTMLSelectElement).value;
  const pathRow = document.getElementById("guard-add-path-row")!;
  const valueTypeRow = document.getElementById("guard-add-value-type-row")!;
  const defaultRow = document.getElementById("guard-add-default-row")!;

  const isToml = mode === "toml_key" || mode === "toml_absent";
  // file_overwrite / markdown_block 固定为 text 类型，无需选值类型
  pathRow.classList.toggle("hidden", !isToml);
  valueTypeRow.classList.toggle("hidden", !isToml);
  defaultRow.classList.toggle("hidden",
    isToml && (document.getElementById("guard-add-value-type") as HTMLSelectElement).value === "none");
}

export function onGuardAddValueTypeChange(): void {
  const vt = (document.getElementById("guard-add-value-type") as HTMLSelectElement).value;
  const defaultRow = document.getElementById("guard-add-default-row")!;
  defaultRow.classList.toggle("hidden", vt === "none");

  const defaultEl = document.getElementById("guard-add-default") as HTMLInputElement;
  const defaultRowParent = defaultEl.parentElement!;
  // 替换 input 为 textarea 或反之
  if (vt === "text" && defaultEl.tagName === "INPUT") {
    const ta = document.createElement("textarea");
    ta.className = "guard-form-textarea";
    ta.id = "guard-add-default";
    ta.value = defaultEl.value;
    defaultRowParent.replaceChild(ta, defaultEl);
  } else if (vt !== "text" && defaultEl.tagName === "TEXTAREA") {
    const inp = document.createElement("input");
    inp.type = "text";
    inp.className = "h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-xs outline-none transition-colors placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";
    inp.id = "guard-add-default";
    inp.value = defaultEl.value;
    defaultRowParent.replaceChild(inp, defaultEl);
  }
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

export async function guardAddCustom(): Promise<void> {
  const id = (document.getElementById("guard-add-id") as HTMLInputElement).value.trim();
  const label = (document.getElementById("guard-add-label") as HTMLInputElement).value.trim();
  const fileSelect = document.getElementById("guard-add-file-select") as HTMLSelectElement;
  const fileId = fileSelect?.value || guardAddParamFileId;
  const mode = (document.getElementById("guard-add-mode") as HTMLSelectElement).value;
  const path = (document.getElementById("guard-add-path") as HTMLInputElement).value.trim();
  const valueType = (document.getElementById("guard-add-value-type") as HTMLSelectElement).value;
  const desc = (document.getElementById("guard-add-desc") as HTMLInputElement).value.trim();
  const defaultEl = document.getElementById("guard-add-default") as HTMLInputElement | HTMLTextAreaElement;
  const defaultRaw = defaultEl.value;

  if (!id) { toast(t("Please enter an ID"), "error"); return; }
  if (!label) { toast(t("Please enter a name"), "error"); return; }
  if (!fileId) { toast(t("Please select a target file"), "error"); return; }
  if ((mode === "toml_key" || mode === "toml_absent") && !path) {
    toast(t("Please enter a TOML path"), "error"); return;
  }

  try {
    const effectiveType = (mode === "file_overwrite" || mode === "markdown_block") ? "text" : valueType;
    const defaultVal = parseDefaultValue(defaultRaw, effectiveType);
    const param = {
      id,
      label,
      description: desc,
      file: "",
      applyMode: mode,
      path,
      valueType: effectiveType,
      default: defaultVal,
      custom: true,
    };
    await invoke("guard_add_custom_param", { param, fileId });
    toast(t("Custom parameter added"), "success");
    // 清空表单并收起
    (document.getElementById("guard-add-id") as HTMLInputElement).value = "";
    (document.getElementById("guard-add-label") as HTMLInputElement).value = "";
    (document.getElementById("guard-add-path") as HTMLInputElement).value = "";
    (document.getElementById("guard-add-desc") as HTMLInputElement).value = "";
    defaultEl.value = "";
    guardAddParamFileId = null;
    closeGuardAddModal();
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Add failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function guardRemoveCustom(id: string): Promise<void> {
  if (!(await ask(t("Delete custom parameter {{id}}?\n\nGuarding stops after deletion. Values already written to ~/.codex/ will not be rolled back; restore manually from ~/.codex/dashi-backups/ if needed.", { id }), { title: t("Delete Custom Parameter"), kind: "warning" }))) {
    return;
  }
  try {
    await invoke("guard_remove_custom_param", { id });
    toast(t("Deleted"), "success");
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Delete failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function guardOpenSchemaFile(): Promise<void> {
  try {
    const path = await invoke<string>("guard_get_schema_file_path");
    // shell 插件的 open 可以打开文件所在目录/文件
    await openUrl(path);
  } catch (e) {
    // 回退：复制路径到剪贴板
    try {
      const path = await invoke<string>("guard_get_schema_file_path");
      await navigator.clipboard.writeText(path);
      toast(t("Path copied to clipboard: {{path}}", { path }), "info");
    } catch {
      toast(t("Open failed: {{error}}", { error: String(e) }), "error");
    }
  }
}
