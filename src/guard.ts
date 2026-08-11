// guard：看守视图渲染与参数操作、看守文件管理、自定义参数管理

import { open as openDialog, ask } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { t } from "./i18n";
import { toast, escapeHtml, fmtTs } from "./core";
import { state } from "./state";
import { GUARD_CONTRACT } from "./generated/guard-contracts";
import type {
  BatchAction,
  BatchRequest,
  BatchScope,
  GuardFile,
  GuardFileFormat,
  GuardParam,
  GuardView,
  JsonValue,
  CapabilitySnapshotDto,
  OperationAuditRecord,
  RoleDetailDto,
  RoleCopyInput,
  RoleDiscoveryDto,
  RoleListDto,
  RoleMigrationDto,
  RoleMigrationResolveInput,
  RoleReorderInput,
  RoleSaveInput,
  SubagentAuditResult,
} from "./generated/guard-contracts";
import {
  guardAddCustomParam as contractAddCustomParam,
  guardAddFile as contractAddFile,
  guardCapabilityGet as contractCapabilityGet,
  guardCapabilityRefresh as contractCapabilityRefresh,
  guardDetectFile as contractDetectFile,
  guardExecuteBatch as contractExecuteBatch,
  guardGroupCreate as contractGroupCreate,
  guardGroupDelete as contractGroupDelete,
  guardGroupRename as contractGroupRename,
  guardGroupReorder as contractGroupReorder,
  guardGetFiles as contractGetFiles,
  guardGetRecoveryStatus as contractGetRecoveryStatus,
  guardGetSchemaFilePath as contractGetSchemaFilePath,
  guardGetView as contractGetView,
  guardPreviewBatch as contractPreviewBatch,
  guardRelativizePickedPath as contractRelativizePickedPath,
  guardRoleAdopt as contractRoleAdopt,
  guardRoleDelete as contractRoleDelete,
  guardRoleCopy as contractRoleCopy,
  guardRoleDiscover as contractRoleDiscover,
  guardRoleGet as contractRoleGet,
  guardRoleList as contractRoleList,
  guardRoleSave as contractRoleSave,
  guardRoleMigrationPlan as contractRoleMigrationPlan,
  guardRoleMigrationResolve as contractRoleMigrationResolve,
  guardRoleReorder as contractRoleReorder,
  guardRoleStopManaging as contractRoleStopManaging,
  guardRunSubagentAudit as contractRunSubagentAudit,
  guardOperationAuditList as contractOperationAuditList,
  guardParameterMove as contractParameterMove,
  guardRemoveCustomParam as contractRemoveCustomParam,
  guardRemoveFile as contractRemoveFile,
  guardRetryRecovery as contractRetryRecovery,
  guardSetEnabled as contractSetEnabled,
  guardSetValue as contractSetValue,
  guardUpdateFile as contractUpdateFile,
} from "./generated/guard-contract-runtime";
import {
  actionOrder,
  createGuardUiState,
  diagnosticMessage,
  reduceGuardUiState,
  shouldConfirmBatch,
  normalizeOperationPhase,
  normalizeProgress,
  operationPhases,
  statusTone,
  type GuardActionState,
  type GuardOperationPhase,
  type GuardUiEvent,
  type GuardUiState,
} from "./guard-view-model";

type GuardFileView = GuardFile;

let guardUiState: GuardUiState = createGuardUiState();
let lastGuardJson = "";
let operationSequence = 0;

type GuardRoleWorkbenchState = {
  managed: RoleListDto | null;
  discovered: RoleDiscoveryDto | null;
  details: Record<string, RoleDetailDto>;
  capability: CapabilitySnapshotDto | null;
  migration: RoleMigrationDto | null;
  operationAudit: OperationAuditRecord[];
  rolesError: string | null;
  discoveryError: string | null;
  capabilityError: string | null;
  migrationError: string | null;
  operationAuditError: string | null;
  busy: string | null;
};

let guardRoleState: GuardRoleWorkbenchState = {
  managed: null,
  discovered: null,
  details: {},
  capability: null,
  migration: null,
  operationAudit: [],
  rolesError: null,
  discoveryError: null,
  capabilityError: null,
  migrationError: null,
  operationAuditError: null,
  busy: null,
};

type RoleFormMode = "create" | "edit" | "copy";
let roleFormMode: RoleFormMode = "create";
let roleFormSource: string | null = null;
let roleFormDraft: RoleSaveInput | null = null;
let guardModalReturnFocus: HTMLElement | null = null;

const ROLE_AUDIT_CODES: Record<string, string> = {
  match: "Match",
  mismatch: "Mismatch",
  ambiguous: "Ambiguous",
  incomplete: "Incomplete",
  unsupported: "Unsupported",
  missing: "Missing",
  success: "Success",
  failed: "Failed",
  rejected: "Rejected",
  busy: "Busy",
  rolled_back: "Rolled back",
  critical: "Critical",
  unsupported_source: "Unsupported source",
  source_unavailable: "Source unavailable",
  parent_dispatch_missing: "Parent dispatch missing",
  client_evidence_missing: "Client evidence missing",
  outbound_evidence_missing: "Outbound evidence missing",
};

const DIAGNOSTIC_LABELS: Record<string, string> = {
  invalid_utf_8: "Invalid UTF-8",
  nul_byte: "NUL byte",
  toml_invalid: "Invalid TOML",
  json_invalid: "Invalid JSON",
  json_duplicate_key: "Duplicate JSON key",
  markdown_malformed_marker: "Malformed Markdown marker",
  markdown_duplicate_marker: "Duplicate Markdown marker",
  markdown_crossing_marker: "Crossing Markdown marker",
  markdown_unmatched_marker: "Unmatched Markdown marker",
  markdown_unclosed_marker: "Unclosed Markdown marker",
  plan_empty_members: "Empty operation members",
  plan_unknown_mode: "Unknown apply mode",
  plan_mode_incompatible: "Incompatible apply mode",
  plan_invalid_path: "Invalid relative path",
  plan_expected_type_mismatch: "Expected type mismatch",
  plan_conflict: "Operation conflict",
};

const OPERATION_PHASE_LABELS: Record<GuardOperationPhase, string> = {
  preflight: "Preflight",
  snapshot: "Snapshot",
  write: "Write",
  verify: "Verify",
  completed: "Completed",
  recovery: "Recovery",
};

const OPERATION_AUDIT_PHASE_LABELS: Record<string, string> = {
  preflight: "Preflight",
  snapshot: "Snapshot",
  write: "Write",
  verify: "Verify",
  completed: "Completed",
  rolled_back: "Rolled back",
  recovery: "Recovery",
  audit: "Audit",
};

const ROLE_MIGRATION_STATUS_LABELS: Record<string, string> = {
  pending: "Pending",
  resolved: "Resolved",
  blocked: "Blocked",
  unavailable: "Unavailable",
};

function localCode(value: unknown, fallback = "Unknown"): string {
  const key = typeof value === "string" ? value : "";
  return t(ROLE_AUDIT_CODES[key] ?? fallback);
}

function diagnosticLabel(value: unknown): string {
  return t(DIAGNOSTIC_LABELS[typeof value === "string" ? value : ""] ?? "Validation diagnostic");
}

function operationPhaseLabel(value: GuardOperationPhase): string {
  return t(OPERATION_PHASE_LABELS[value] ?? "Unknown");
}

function operationAuditPhaseLabel(value: unknown): string {
  return t(OPERATION_AUDIT_PHASE_LABELS[typeof value === "string" ? value : ""] ?? "Unknown");
}

function roleMigrationStatusLabel(value: unknown): string {
  return t(ROLE_MIGRATION_STATUS_LABELS[typeof value === "string" ? value : ""] ?? "Unknown");
}

function guardWriteBlocked(): boolean {
  return Boolean(guardUiState.batch?.busy || guardUiState.recovery?.blocked);
}

function rememberFocus(): void {
  const active = document.activeElement;
  if (active instanceof HTMLElement) guardModalReturnFocus = active;
}

function focusFirstControl(root: HTMLElement | null): void {
  if (!root) return;
  const control = root.querySelector<HTMLElement>("input:not([disabled]), select:not([disabled]), textarea:not([disabled]), button:not([disabled])");
  control?.focus();
}

function showGuardModal(id: string): void {
  const modal = document.getElementById(id);
  if (!modal) return;
  rememberFocus();
  modal.classList.remove("hidden");
  focusFirstControl(modal);
}

function hideGuardModal(id: string): void {
  const modal = document.getElementById(id);
  modal?.classList.add("hidden");
  guardModalReturnFocus?.focus();
  guardModalReturnFocus = null;
}

/** Called by shell for keyboard Escape handling. */
export function closeGuardModalOnEscape(): boolean {
  for (const id of ["guard-role-modal", "guard-group-modal", "guard-add-modal", "guard-file-modal"]) {
    const modal = document.getElementById(id);
    if (modal && !modal.classList.contains("hidden")) {
      hideGuardModal(id);
      if (id === "guard-role-modal") roleFormDraft = null;
      if (id === "guard-group-modal") guardGroupFormId = null;
      return true;
    }
  }
  return false;
}

export function getGuardUiState(): GuardUiState {
  return guardUiState;
}

function dispatchGuard(event: GuardUiEvent): void {
  guardUiState = reduceGuardUiState(guardUiState, event);
}

function nextOperationId(): string {
  operationSequence += 1;
  return `guard-operation-${Date.now()}-${operationSequence}`;
}

// ============ 总开关 ============
export async function toggleGuard(): Promise<void> {
  if (guardWriteBlocked()) return;
  const enabled = !state.guardState.enabled;
  try {
    await contractSetEnabled(enabled);
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
  const btn = document.getElementById("btn-guard");
  // Guard stays reachable while disabled so recovery and migration remain visible.
  if (btn) btn.classList.remove("hidden");
}

// ============ 看守视图 ============
export async function refreshGuardView(force = false): Promise<void> {
  const viewEl = document.getElementById("guard-view");
  if (!viewEl || viewEl.classList.contains("hidden")) return;
  try {
    const view = await contractGetView();
    const json = JSON.stringify(view);
    if (!force && json === lastGuardJson) {
      renderGuardWorkbench(false);
      return;
    }
    lastGuardJson = json;
    dispatchGuard({ type: "view/success", view });
    state.guardState.enabled = view.enabled;
    renderGuardWorkbench();
    renderGuardToggle();
  } catch (error) {
    dispatchGuard({ type: "view/error", error: String(error) });
    renderGuardWorkbench();
  }
}

function roleLifecycleLabel(value: string): string {
  const labels: Record<string, string> = { disabled: "Disabled", applied: "Applied", locked: "Locked", mixed: "Mixed" };
  return t(labels[value] ?? "Unknown");
}

function roleHealthLabel(value: string): string {
  const labels: Record<string, string> = { healthy: "Healthy", drifted: "Drifted", invalid: "Invalid", unsupported: "Unsupported", error: "Error" };
  return t(labels[value] ?? "Unknown");
}

function roleCapabilityLabel(value: string): string {
  const labels: Record<string, string> = {
    supported: "Supported",
    unsupported_model: "Unsupported model",
    unsupported_reasoning_effort: "Unsupported reasoning effort",
    unsupported_multi_agent_version: "Unsupported multi-agent version",
    unavailable: "Unavailable",
    invalid: "Invalid",
  };
  return t(labels[value] ?? "Unknown");
}

function fmtMs(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "—";
  return new Date(value).toLocaleString(currentLanguageForGuard(), { hour12: false });
}

function currentLanguageForGuard(): string {
  return document.documentElement.lang === "zh-CN" ? "zh-CN" : "en-US";
}

function roleBusyButton(action: string, id: string, label: string, danger = false, blocked = false): string {
  const writeAction = ["guardRoleAdopt", "guardRoleStopManaging", "guardRoleDelete", "guardRoleEdit", "guardRoleCopy", "guardRoleMoveUp", "guardRoleMoveDown"].includes(action);
  const disabled = guardRoleState.busy || blocked || (writeAction && guardWriteBlocked()) ? " disabled" : "";
  return `<button class="guard-role-button${danger ? " is-danger" : ""}" data-action="${action}" data-role-id="${escapeHtml(id)}"${disabled}>${escapeHtml(t(label))}</button>`;
}

function renderRoleDetails(role: RoleDetailDto, id: string): string {
  return `<div class="guard-role-details" id="guard-role-details-${escapeHtml(id)}">
    <div class="guard-role-detail-grid">
      <div><span class="guard-role-detail-label">${escapeHtml(t("Purpose"))}</span><span>${escapeHtml(role.purpose || t("Not provided"))}</span></div>
      <div><span class="guard-role-detail-label">${escapeHtml(t("Selection criteria"))}</span><span>${escapeHtml(role.selectionCriteria || t("Not provided"))}</span></div>
      <div><span class="guard-role-detail-label">${escapeHtml(t("Model"))}</span><code>${escapeHtml(role.model)}</code></div>
      <div><span class="guard-role-detail-label">${escapeHtml(t("Reasoning effort"))}</span><code>${escapeHtml(role.effort)}</code></div>
      <div><span class="guard-role-detail-label">${escapeHtml(t("Policy revision"))}</span><code>${escapeHtml(String(role.policyRevision))}</code></div>
      <div><span class="guard-role-detail-label">${escapeHtml(t("File"))}</span><span>${escapeHtml(role.actualFilePresent ? t("Present") : t("Missing"))}</span></div>
    </div>
    <div class="guard-role-instructions"><span class="guard-role-detail-label">${escapeHtml(t("Instructions"))}</span><pre>${escapeHtml(role.instructions || t("Not provided"))}</pre></div>
  </div>`;
}

function capabilityModel(slug: string): { slug: string; multiAgentVersion: string; efforts: string[] } | null {
  return guardRoleState.capability?.models.find((model) => model.slug === slug) ?? null;
}

function renderRoleFormOptions(): void {
  const modelSelect = document.getElementById("guard-role-model") as HTMLSelectElement | null;
  const effortSelect = document.getElementById("guard-role-effort") as HTMLSelectElement | null;
  const warning = document.getElementById("guard-role-capability-warning");
  if (!modelSelect || !effortSelect) return;
  const draft = roleFormDraft;
  const capability = guardRoleState.capability;
  const models = capability?.models ?? [];
  const selectedModel = draft?.model ?? modelSelect.value;
  modelSelect.innerHTML = models.map((model) => `<option value="${escapeHtml(model.slug)}">${escapeHtml(model.slug)}</option>`).join("");
  if (selectedModel && !models.some((model) => model.slug === selectedModel)) {
    modelSelect.insertAdjacentHTML("afterbegin", `<option value="${escapeHtml(selectedModel)}">${escapeHtml(selectedModel)} (${escapeHtml(t("Unavailable"))})</option>`);
  }
  modelSelect.value = selectedModel;
  const model = capabilityModel(selectedModel);
  const efforts = model?.efforts ?? [];
  const selectedEffort = draft?.effort ?? effortSelect.value;
  effortSelect.innerHTML = efforts.map((effort) => `<option value="${escapeHtml(effort)}">${escapeHtml(effort)}</option>`).join("");
  if (selectedEffort && !efforts.includes(selectedEffort)) {
    effortSelect.insertAdjacentHTML("afterbegin", `<option value="${escapeHtml(selectedEffort)}">${escapeHtml(selectedEffort)} (${escapeHtml(t("Unavailable"))})</option>`);
  }
  effortSelect.value = selectedEffort;
  const unavailable = !capability?.available || !model || !efforts.includes(selectedEffort);
  modelSelect.disabled = models.length === 0;
  effortSelect.disabled = !model || efforts.length === 0;
  if (warning) {
    warning.textContent = unavailable ? t("Capability must be refreshed before saving a role") : "";
    warning.classList.toggle("hidden", !unavailable);
  }
}

export function handleGuardRoleModelChange(): void {
  const draft = readRoleFormDraft();
  roleFormDraft = draft;
  renderRoleFormOptions();
}

function setRoleFormDraft(draft: RoleSaveInput): void {
  roleFormDraft = { ...draft };
  const values: Record<string, string> = {
    id: draft.id,
    displayName: draft.displayName,
    purpose: draft.purpose,
    selectionCriteria: draft.selectionCriteria,
    model: draft.model,
    effort: draft.effort,
    instructions: draft.instructions,
  };
  for (const [name, value] of Object.entries(values)) {
    const input = document.getElementById(`guard-role-${name}`) as HTMLInputElement | HTMLTextAreaElement | null;
    if (input) input.value = value;
  }
  renderRoleFormOptions();
}

function readRoleFormDraft(): RoleSaveInput {
  const value = (id: string): string => (document.getElementById(id) as HTMLInputElement | HTMLTextAreaElement | null)?.value.trim() ?? "";
  return {
    id: value("guard-role-id"),
    displayName: value("guard-role-displayName"),
    purpose: value("guard-role-purpose"),
    selectionCriteria: value("guard-role-selectionCriteria"),
    model: (document.getElementById("guard-role-model") as HTMLSelectElement | null)?.value ?? "",
    effort: (document.getElementById("guard-role-effort") as HTMLSelectElement | null)?.value ?? "",
    instructions: value("guard-role-instructions"),
  };
}

function renderRoleFormTitle(): void {
  const title = document.getElementById("guard-role-modal-title");
  const submit = document.getElementById("guard-role-submit");
  const key = roleFormMode === "create" ? "Create role" : roleFormMode === "copy" ? "Copy role" : "Edit role";
  if (title) title.textContent = t(key);
  if (submit) submit.textContent = t(roleFormMode === "create" ? "Create" : roleFormMode === "copy" ? "Copy" : "Save");
}

function openRoleForm(mode: RoleFormMode, draft: RoleSaveInput, source: string | null = null): void {
  roleFormMode = mode;
  roleFormSource = source;
  renderRoleFormTitle();
  showGuardModal("guard-role-modal");
  setRoleFormDraft(draft);
}

export function openGuardRoleCreate(): void {
  const model = guardRoleState.capability?.models[0];
  openRoleForm("create", {
    id: "",
    displayName: "",
    purpose: "",
    selectionCriteria: "",
    model: model?.slug ?? "",
    effort: model?.efforts[0] ?? "",
    instructions: "",
  });
}

export async function guardRoleEdit(id: string): Promise<void> {
  if (guardWriteBlocked() || guardRoleState.busy) return;
  try {
    const detail = guardRoleState.details[id] ?? await contractRoleGet(id);
    guardRoleState = { ...guardRoleState, details: { ...guardRoleState.details, [id]: detail } };
    openRoleForm("edit", {
      id: detail.id,
      displayName: detail.displayName,
      purpose: detail.purpose,
      selectionCriteria: detail.selectionCriteria,
      model: detail.model,
      effort: detail.effort,
      instructions: detail.instructions,
    }, id);
  } catch (error) {
    toast(t("Role action failed: {{error}}", { error: localCode(error, "Operation unavailable") }), "error");
  }
}

export async function guardRoleCopy(id: string): Promise<void> {
  if (guardWriteBlocked() || guardRoleState.busy) return;
  try {
    const detail = guardRoleState.details[id] ?? await contractRoleGet(id);
    guardRoleState = { ...guardRoleState, details: { ...guardRoleState.details, [id]: detail } };
    openRoleForm("copy", {
      id: `${detail.id}-copy`,
      displayName: `${detail.displayName} ${t("Copy suffix")}`,
      purpose: detail.purpose,
      selectionCriteria: detail.selectionCriteria,
      model: detail.model,
      effort: detail.effort,
      instructions: detail.instructions,
    }, id);
  } catch (error) {
    toast(t("Role action failed: {{error}}", { error: localCode(error, "Operation unavailable") }), "error");
  }
}

export function closeGuardRoleModal(): void {
  roleFormDraft = null;
  roleFormSource = null;
  hideGuardModal("guard-role-modal");
}

export async function saveGuardRoleForm(): Promise<void> {
  if (guardWriteBlocked() || guardRoleState.busy) return;
  const draft = readRoleFormDraft();
  roleFormDraft = draft;
  if (!draft.id || !/^[a-z][a-z0-9-]{0,62}$/.test(draft.id)) {
    toast(t("Please enter a valid role ID"), "error");
    return;
  }
  if (!draft.displayName) {
    toast(t("Please enter a role display name"), "error");
    return;
  }
  const model = capabilityModel(draft.model);
  if (!guardRoleState.capability?.available || !model || !model.efforts.includes(draft.effort)) {
    renderRoleFormOptions();
    toast(t("Capability must be refreshed before saving a role"), "error");
    return;
  }
  try {
    guardRoleState = { ...guardRoleState, busy: "role-save", rolesError: null };
    renderGuardRoleWorkbench();
    if (roleFormMode === "copy" && roleFormSource) {
      await contractRoleCopy(roleFormSource, {
        id: draft.id,
        displayName: draft.displayName,
        selectionCriteria: draft.selectionCriteria,
      } satisfies RoleCopyInput);
    } else {
      await contractRoleSave(draft);
    }
    guardRoleState = { ...guardRoleState, busy: null };
    closeGuardRoleModal();
    toast(t(roleFormMode === "copy" ? "Role copied" : roleFormMode === "create" ? "Role created" : "Role saved"), "success");
    await refreshGuardRoles();
  } catch (error) {
    guardRoleState = { ...guardRoleState, busy: null, rolesError: localCode(error, "Operation unavailable") };
    renderGuardRoleWorkbench();
    toast(t("Role action failed: {{error}}", { error: localCode(error, "Operation unavailable") }), "error");
  }
}

function renderManagedRoles(): void {
  const container = document.getElementById("guard-managed-roles");
  if (!container) return;
  if (guardRoleState.rolesError) {
    container.innerHTML = `<div class="guard-inline-error">${escapeHtml(guardRoleState.rolesError)}</div>`;
    return;
  }
  const roles = guardRoleState.managed?.roles ?? [];
  if (guardRoleState.managed === null) {
    container.innerHTML = `<div class="guard-role-create-row"><button class="guard-role-button" data-action="guardRoleCreate">${escapeHtml(t("Create role"))}</button></div><span class="guard-empty-diagnostic">${escapeHtml(t("Loading roles"))}</span>`;
    return;
  }
  if (roles.length === 0) {
    container.innerHTML = `<div class="guard-role-create-row"><button class="guard-role-button" data-action="guardRoleCreate">${escapeHtml(t("Create role"))}</button></div><span class="guard-empty-diagnostic">${escapeHtml(t("No managed roles"))}</span>`;
    return;
  }
  const roleCards = roles.map((role) => {
    const expanded = guardUiState.expandedRoles.includes(role.id);
    const detail = guardRoleState.details[role.id];
    const protectedRole = role.id === "default";
    const roleScope: BatchScope = { role: { roleId: role.id } };
    const roleActionReasons = actionReasons(role, roleScope);
    const detailId = `guard-role-details-${escapeHtml(role.id)}`;
    const details = detail
      ? renderRoleDetails(detail, role.id)
      : `<div class="guard-role-details guard-empty-diagnostic" id="${detailId}">${escapeHtml(t("Loading role details"))}</div>`;
    const detailsMarkup = expanded ? details : details.replace("<div ", "<div hidden ");
    return `<article class="guard-role-card" data-role-id="${escapeHtml(role.id)}">
      <div class="guard-role-summary">
        <button class="guard-role-disclosure" data-action="guardRoleToggle" data-role-id="${escapeHtml(role.id)}" aria-expanded="${String(expanded)}" aria-controls="${detailId}"><span class="guard-role-chevron">${expanded ? "▾" : "▸"}</span><span class="guard-role-name">${escapeHtml(role.displayName || role.id)}</span><code>${escapeHtml(role.id)}</code></button>
        <div class="guard-role-axes"><span class="guard-role-axis ${toneClass(role.lifecycle)}"><span>${escapeHtml(t("Lifecycle"))}</span>${escapeHtml(roleLifecycleLabel(role.lifecycle))}</span><span class="guard-role-axis ${toneClass(role.health)}"><span>${escapeHtml(t("Health"))}</span>${escapeHtml(roleHealthLabel(role.health))}</span></div>
        <div class="guard-role-actions">${roleBusyButton("guardRoleToggle", role.id, expanded ? "Collapse" : "Details")}${roleBusyButton("guardRoleEdit", role.id, "Edit role")}${roleBusyButton("guardRoleCopy", role.id, "Copy role")}${roleBusyButton("guardRoleMoveUp", role.id, "Move up", false, protectedRole || role.order <= 1)}${roleBusyButton("guardRoleMoveDown", role.id, "Move down", false, protectedRole || role.order >= (guardRoleState.managed?.roles.length ?? 1) - 1)}${roleBusyButton("guardRoleStopManaging", role.id, "Stop managing", false, protectedRole)}${roleBusyButton("guardRoleDelete", role.id, "Delete", true, protectedRole)}</div>
      </div>
      <div class="guard-role-batch-actions"><div class="guard-action-bar">${actionOrder.map((action) => actionButton(roleScope, action, role)).join("")}</div>${roleActionReasons ? `<div class="guard-action-reasons">${escapeHtml(roleActionReasons)}</div>` : ""}</div>
      ${detailsMarkup}
    </article>`;
  }).join("");
  container.innerHTML = `<div class="guard-role-create-row"><button class="guard-role-button" data-action="guardRoleCreate">${escapeHtml(t("Create role"))}</button></div>${roleCards}`;
}

function renderDiscoveredRoles(): void {
  const container = document.getElementById("guard-discovered-roles");
  if (!container) return;
  if (guardRoleState.discoveryError) {
    container.innerHTML = `<div class="guard-inline-error">${escapeHtml(guardRoleState.discoveryError)}</div>`;
    return;
  }
  const roles = guardRoleState.discovered?.roles ?? [];
  if (guardRoleState.discovered === null) {
    container.innerHTML = `<span class="guard-empty-diagnostic">${escapeHtml(t("Loading discovered roles"))}</span>`;
    return;
  }
  if (roles.length === 0) {
    container.innerHTML = `<span class="guard-empty-diagnostic">${escapeHtml(t("No discovered roles"))}</span>`;
    return;
  }
  container.innerHTML = roles.map((role) => {
    const diagnostics = role.diagnostics.map((diagnostic) => `<span class="guard-role-diagnostic">${escapeHtml(diagnosticLabel(diagnostic.code))}${diagnostic.field ? ` · ${escapeHtml(diagnostic.field)}` : ""}</span>`).join("");
    const adopt = role.canAdopt && !role.managed ? roleBusyButton("guardRoleAdopt", role.id, "Adopt") : "";
    return `<div class="guard-discovered-role"><div class="guard-discovered-main"><strong>${escapeHtml(role.id)}</strong><span class="guard-role-file">${escapeHtml(role.relativeFileName)}</span><span class="guard-role-axis ${toneClass(role.capability)}"><span>${escapeHtml(t("Capability"))}</span>${escapeHtml(roleCapabilityLabel(role.capability))}</span></div><div class="guard-discovered-meta"><span>${escapeHtml(role.managed ? t("Managed") : role.syntax === "valid" ? t("Ready to adopt") : t("Not adoptable"))}</span>${diagnostics}</div>${adopt}</div>`;
  }).join("");
}

function renderGuardCapability(): void {
  const container = document.getElementById("guard-capability-content");
  if (!container) return;
  if (guardRoleState.capabilityError) {
    container.innerHTML = `<div class="guard-inline-error">${escapeHtml(guardRoleState.capabilityError)}</div>`;
    return;
  }
  const capability = guardRoleState.capability;
  if (!capability) {
    container.innerHTML = `<span class="guard-empty-diagnostic">${escapeHtml(t("Capability snapshot unavailable"))}</span>`;
    return;
  }
  const models = capability.models.map((model) => `<div class="guard-capability-model"><strong>${escapeHtml(model.slug)}</strong><span>${escapeHtml(model.multiAgentVersion)}</span><span>${escapeHtml(model.efforts.join(", ") || t("No reasoning levels"))}</span></div>`).join("");
  const capabilityStatus = capability.available ? t("Available") : capability.error ? t("Error") : t("Unavailable");
  container.innerHTML = `<div class="guard-capability-status"><span class="guard-role-axis ${capability.available ? "guard-tone-positive" : "guard-tone-negative"}"><span>${escapeHtml(t("Status"))}</span>${escapeHtml(capabilityStatus)}</span>${capability.executableIdentity ? `<code>${escapeHtml(capability.executableIdentity)}</code>` : ""}<span class="guard-capability-fetched">${escapeHtml(t("Fetched {{time}}", { time: fmtMs(capability.fetchedAtMs) }))}</span>${capability.error ? `<span class="guard-role-diagnostic">${escapeHtml(localCode(capability.error, "Capability unavailable"))}</span>` : ""}</div>${models ? `<div class="guard-capability-models">${models}</div>` : `<span class="guard-empty-diagnostic">${escapeHtml(t("No models reported"))}</span>`}`;
}

function renderGuardMigration(): void {
  const container = document.getElementById("guard-migration-result");
  if (!container) return;
  if (guardRoleState.migrationError) {
    container.innerHTML = `<div class="guard-inline-error">${escapeHtml(guardRoleState.migrationError)}</div>`;
    return;
  }
  const migration = guardRoleState.migration;
  if (!migration) {
    container.innerHTML = `<span class="guard-empty-diagnostic">${escapeHtml(t("No migration plan"))}</span>`;
    return;
  }
  const role = migration.role ? `<div class="guard-migration-role">${escapeHtml(t("Resolved role"))}: <strong>${escapeHtml(migration.role.displayName)}</strong> <code>${escapeHtml(migration.role.id)}</code></div>` : "";
  const choices = migration.choices.map((choice) => `<button class="guard-role-button" data-action="guardRoleMigrationResolve" data-choice="${escapeHtml(choice)}">${escapeHtml(t(choice === "adopt_live" ? "Adopt live file" : choice === "adopt_stored" ? "Adopt stored policy" : "Use safe default"))}</button>`).join("");
  container.innerHTML = `<div class="guard-migration-status"><span class="guard-role-axis ${toneClass(migration.status)}"><span>${escapeHtml(t("Status"))}</span>${escapeHtml(roleMigrationStatusLabel(migration.status))}</span>${migration.reason ? `<span>${escapeHtml(t("Reason"))}: ${escapeHtml(migration.reason)}</span>` : ""}</div>${role}${choices ? `<div class="guard-action-bar">${choices}</div>` : ""}`;
}

function renderGuardRuntimeAudit(): void {
  const container = document.getElementById("guard-runtime-audit");
  if (!container) return;
  const audit = guardUiState.audit as SubagentAuditResult | null;
  if (guardUiState.auditError) {
    container.innerHTML = `<div class="guard-inline-error">${escapeHtml(guardUiState.auditError)}</div>`;
    return;
  }
  if (!audit) {
    container.innerHTML = `<span class="guard-empty-diagnostic">${escapeHtml(t("No runtime audit yet"))}</span>`;
    return;
  }
  const agents = audit.agents.map((agent) => {
    const parent = agent.parent;
    const parentEvidence = [
      parent.observedAgentType ? `${t("Agent type")}: ${parent.observedAgentType}` : "",
      parent.observedForkTurns ? `${t("Fork turns")}: ${parent.observedForkTurns}` : "",
      parent.overrideModel ? `${t("Override model")}: ${parent.overrideModel}` : "",
      parent.overrideEffort ? `${t("Override effort")}: ${parent.overrideEffort}` : "",
    ].filter(Boolean).join(" · ");
    const turns = agent.turns.map((turn) => `<div class="guard-audit-turn"><span>${escapeHtml(turn.model ?? t("Model unavailable"))} / ${escapeHtml(turn.effort ?? t("Effort unavailable"))}</span><span>${escapeHtml(turn.multiAgentVersion ?? t("V2 evidence unavailable"))}</span><span class="guard-role-axis ${toneClass(turn.verdict)}"><span>${escapeHtml(t("Turn"))}</span>${escapeHtml(localCode(turn.verdict))}</span><span class="guard-audit-outbound">${escapeHtml(turn.outboundRequestedModel ? `${t("Outbound requested model")}: ${turn.outboundRequestedModel}` : t("Outbound evidence unavailable"))}</span></div>`).join("");
    return `<article class="guard-audit-agent"><div class="guard-audit-agent-heading"><strong>${escapeHtml(agent.roleId)}</strong><span class="guard-role-axis ${toneClass(agent.verdict)}"><span>${escapeHtml(t("Verdict"))}</span>${escapeHtml(localCode(agent.verdict))}</span></div><div class="guard-audit-evidence"><span>${escapeHtml(t("Parent dispatch"))}: ${escapeHtml(parentEvidence || t("Parent dispatch evidence unavailable"))}</span><span>${escapeHtml(t("Expected model"))}: ${escapeHtml(agent.expectedModel ?? t("Unknown"))} · ${escapeHtml(t("Expected effort"))}: ${escapeHtml(agent.expectedEffort ?? t("Unknown"))}</span></div>${turns ? `<div class="guard-audit-turns">${turns}</div>` : `<span class="guard-empty-diagnostic">${escapeHtml(t("No turns sampled"))}</span>`}</article>`;
  }).join("");
  const notices = audit.notices.map((notice) => `<span class="guard-role-diagnostic">${escapeHtml(localCode(notice.code, "Audit warning"))}</span>`).join("");
  container.innerHTML = `<div class="guard-audit-heading"><span class="guard-role-axis ${toneClass(audit.sourceSupport)}"><span>${escapeHtml(t("Source"))}</span>${escapeHtml(audit.sourceSupport === "full" ? t("Full evidence") : audit.sourceSupport === "rollout_only" ? t("Rollout evidence only") : t("Unsupported"))}</span><span>${escapeHtml(t("Checked {{time}}", { time: fmtMs(audit.checkedAtMs) }))}</span><span class="guard-audit-privacy">${escapeHtml(t("Server actual model is not observable"))}</span></div>${agents ? `<div class="guard-audit-agents">${agents}</div>` : `<span class="guard-empty-diagnostic">${escapeHtml(t("No agents sampled"))}</span>`}${notices ? `<div class="guard-audit-notices">${notices}</div>` : ""}`;
}

function renderGuardOperationAudit(): void {
  const container = document.getElementById("guard-operation-audit");
  if (!container) return;
  if (guardRoleState.operationAuditError) {
    container.innerHTML = `<div class="guard-inline-error">${escapeHtml(guardRoleState.operationAuditError)}</div>`;
    return;
  }
  if (guardRoleState.operationAudit.length === 0) {
    container.innerHTML = `<span class="guard-empty-diagnostic">${escapeHtml(t("No operation audit records"))}</span>`;
    return;
  }
  container.innerHTML = guardRoleState.operationAudit.slice().reverse().map((record) => `<div class="guard-operation-audit-row"><span class="guard-operation-audit-time">${escapeHtml(fmtMs(record.atMs))}</span><code>${escapeHtml(record.scope)}</code><span>${escapeHtml(operationAuditPhaseLabel(record.phase))}</span><span class="guard-role-axis ${toneClass(record.result)}">${escapeHtml(localCode(record.result))}</span><span class="guard-operation-counts">${escapeHtml(t("{{changed}} changed · {{unchanged}} unchanged · {{files}} files", { changed: record.changed, unchanged: record.unchanged, files: record.files }))}</span>${record.errorCode ? `<span class="guard-role-diagnostic">${escapeHtml(localCode(record.errorCode, "Operation warning"))}</span>` : ""}</div>`).join("");
}

function renderGuardRoleWorkbench(): void {
  renderManagedRoles();
  renderDiscoveredRoles();
  renderGuardCapability();
  renderGuardMigration();
  renderGuardRuntimeAudit();
  renderGuardOperationAudit();
  const root = document.getElementById("guard-view");
  root?.querySelectorAll<HTMLButtonElement>('[data-action="guardRoleRefresh"], [data-action="guardRoleDiscoverRefresh"], [data-action="guardCapabilityRefresh"], [data-action="guardRoleMigrationPlan"], [data-action="guardRunRuntimeAudit"], [data-action="guardOperationAuditRefresh"]').forEach((button) => {
    button.disabled = Boolean(guardRoleState.busy);
  });
}

export async function refreshGuardRoles(): Promise<void> {
  try {
    guardRoleState = { ...guardRoleState, busy: "roles", rolesError: null, discoveryError: null };
    renderGuardRoleWorkbench();
    const [managed, discovered] = await Promise.all([contractRoleList(), contractRoleDiscover()]);
    guardRoleState = { ...guardRoleState, managed, discovered, busy: null, rolesError: null, discoveryError: null };
  } catch (error) {
    const message = guardErrorLabel(error);
    guardRoleState = { ...guardRoleState, busy: null, rolesError: message, discoveryError: message };
  }
  renderGuardRoleWorkbench();
}

export async function guardRoleMove(id: string, direction: "up" | "down"): Promise<void> {
  if (guardRoleState.busy || guardWriteBlocked()) return;
  const roles = guardRoleState.managed?.roles ?? [];
  const index = roles.findIndex((role) => role.id === id);
  if (index < 0) return;
  const target = direction === "up" ? index - 1 : index + 1;
  if (target < 0 || target >= roles.length || roles[index].id === "default" || roles[target].id === "default") return;
  const ids = roles.map((role) => role.id);
  [ids[index], ids[target]] = [ids[target], ids[index]];
  try {
    guardRoleState = { ...guardRoleState, busy: "role-reorder", rolesError: null };
    renderGuardRoleWorkbench();
    const managed = await contractRoleReorder({ ids } satisfies RoleReorderInput);
    guardRoleState = { ...guardRoleState, managed, busy: null };
    toast(t("Role order updated"), "success");
  } catch (error) {
    guardRoleState = { ...guardRoleState, busy: null, rolesError: guardErrorLabel(error) };
    renderGuardRoleWorkbench();
    toast(t("Role action failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
  renderGuardRoleWorkbench();
}

export async function refreshGuardRoleDiscovery(): Promise<void> {
  try {
    guardRoleState = { ...guardRoleState, busy: "discovery", discoveryError: null };
    renderGuardRoleWorkbench();
    const discovered = await contractRoleDiscover();
    guardRoleState = { ...guardRoleState, discovered, busy: null };
  } catch (error) {
    guardRoleState = { ...guardRoleState, busy: null, discoveryError: guardErrorLabel(error) };
  }
  renderGuardRoleWorkbench();
}

export async function toggleGuardRole(id: string): Promise<void> {
  const expanded = guardUiState.expandedRoles.includes(id);
  dispatchGuard({ type: "role/toggle", roleId: id });
  renderGuardRoleWorkbench();
  if (!expanded && !guardRoleState.details[id]) {
    try {
      const detail = await contractRoleGet(id);
      guardRoleState = { ...guardRoleState, details: { ...guardRoleState.details, [id]: detail } };
    } catch (error) {
      guardRoleState = { ...guardRoleState, rolesError: guardErrorLabel(error) };
    }
    renderGuardRoleWorkbench();
  }
}

export async function guardRoleAdopt(id: string): Promise<void> {
  if (guardRoleState.busy || guardWriteBlocked()) return;
  if (!(await ask(t("Adopt discovered role {{id}}?", { id }), { title: t("Adopt role"), kind: "warning" }))) return;
  try {
    guardRoleState = { ...guardRoleState, busy: "adopt", rolesError: null, discoveryError: null };
    renderGuardRoleWorkbench();
    const detail = await contractRoleAdopt(id);
    guardRoleState = { ...guardRoleState, details: { ...guardRoleState.details, [id]: detail }, busy: null };
    toast(t("Role adopted"), "success");
    await refreshGuardRoles();
  } catch (error) {
    guardRoleState = { ...guardRoleState, busy: null, rolesError: guardErrorLabel(error) };
    renderGuardRoleWorkbench();
    toast(t("Role action failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
}

export async function guardRoleStopManaging(id: string): Promise<void> {
  if (guardRoleState.busy || guardWriteBlocked()) return;
  try {
    guardRoleState = { ...guardRoleState, busy: "stop-managing", rolesError: null, discoveryError: null };
    renderGuardRoleWorkbench();
    await contractRoleStopManaging(id);
    guardRoleState = { ...guardRoleState, busy: null };
    toast(t("Role management stopped"), "info");
    await refreshGuardRoles();
  } catch (error) {
    guardRoleState = { ...guardRoleState, busy: null, rolesError: guardErrorLabel(error) };
    renderGuardRoleWorkbench();
    toast(t("Role action failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
}

export async function guardRoleDelete(id: string): Promise<void> {
  if (guardRoleState.busy || guardWriteBlocked()) return;
  if (!(await ask(t("Delete managed role {{id}}? The role file will be backed up before removal.", { id }), { title: t("Delete role"), kind: "warning" }))) return;
  try {
    guardRoleState = { ...guardRoleState, busy: "delete", rolesError: null, discoveryError: null };
    renderGuardRoleWorkbench();
    const report = await contractRoleDelete(id);
    const details = { ...guardRoleState.details };
    delete details[id];
    guardRoleState = { ...guardRoleState, details, busy: null };
    toast(t("Role deleted: {{id}}", { id: report.id }), "success");
    await refreshGuardRoles();
  } catch (error) {
    guardRoleState = { ...guardRoleState, busy: null, rolesError: guardErrorLabel(error) };
    renderGuardRoleWorkbench();
    toast(t("Role action failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
}

export async function refreshGuardCapability(refresh = false): Promise<void> {
  try {
    guardRoleState = { ...guardRoleState, busy: refresh ? "capability" : guardRoleState.busy, capabilityError: null };
    renderGuardRoleWorkbench();
    const capability = refresh ? await contractCapabilityRefresh() : await contractCapabilityGet();
    guardRoleState = { ...guardRoleState, capability, busy: null };
  } catch (error) {
    guardRoleState = { ...guardRoleState, busy: null, capabilityError: guardErrorLabel(error) };
  }
  renderGuardRoleWorkbench();
}

function safeDefaultToml(): string {
  return (document.getElementById("guard-migration-default") as HTMLTextAreaElement | null)?.value ?? "";
}

export async function planGuardRoleMigration(): Promise<void> {
  try {
    guardRoleState = { ...guardRoleState, busy: "migration", migrationError: null };
    renderGuardRoleWorkbench();
    const migration = await contractRoleMigrationPlan(safeDefaultToml());
    guardRoleState = { ...guardRoleState, migration, busy: null };
  } catch (error) {
    guardRoleState = { ...guardRoleState, busy: null, migrationError: guardErrorLabel(error) };
  }
  renderGuardRoleWorkbench();
}

export async function resolveGuardRoleMigration(choice: string): Promise<void> {
  if (!["adopt_live", "adopt_stored", "use_safe_default"].includes(choice)) return;
  try {
    guardRoleState = { ...guardRoleState, busy: "migration", migrationError: null };
    renderGuardRoleWorkbench();
    const input: RoleMigrationResolveInput = { choice, safeDefaultToml: safeDefaultToml() };
    const migration = await contractRoleMigrationResolve(input);
    guardRoleState = { ...guardRoleState, migration, busy: null };
    toast(t("Role migration resolved"), "success");
    await refreshGuardRoles();
  } catch (error) {
    guardRoleState = { ...guardRoleState, busy: null, migrationError: guardErrorLabel(error) };
    renderGuardRoleWorkbench();
    toast(t("Migration failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
}

export async function runGuardRuntimeAudit(): Promise<void> {
  try {
    guardRoleState = { ...guardRoleState, busy: "audit" };
    renderGuardRoleWorkbench();
    const result = await contractRunSubagentAudit();
    dispatchGuard({ type: "audit/success", result });
  } catch (error) {
    dispatchGuard({ type: "audit/error", error: guardErrorLabel(error) });
  }
  guardRoleState = { ...guardRoleState, busy: null };
  renderGuardRoleWorkbench();
}

export async function refreshGuardOperationAudit(): Promise<void> {
  try {
    const records = await contractOperationAuditList();
    guardRoleState = { ...guardRoleState, operationAudit: records, operationAuditError: null };
  } catch (error) {
    guardRoleState = { ...guardRoleState, operationAuditError: guardErrorLabel(error) };
  }
  renderGuardRoleWorkbench();
}

type GuardGroupFormMode = "create" | "rename";
let guardGroupFormMode: GuardGroupFormMode = "create";
let guardGroupFormId: string | null = null;

export function openGuardGroupCreate(): void {
  guardGroupFormMode = "create";
  guardGroupFormId = null;
  const title = document.getElementById("guard-group-modal-title");
  const submit = document.getElementById("guard-group-submit");
  const input = document.getElementById("guard-group-name") as HTMLInputElement | null;
  if (title) title.textContent = t("Create group");
  if (submit) submit.textContent = t("Create");
  if (input) input.value = "";
  showGuardModal("guard-group-modal");
}

export function openGuardGroupRename(id: string): void {
  const group = guardUiState.view?.groups.find((candidate) => candidate.id === id);
  if (!group || group.builtin) return;
  guardGroupFormMode = "rename";
  guardGroupFormId = id;
  const title = document.getElementById("guard-group-modal-title");
  const submit = document.getElementById("guard-group-submit");
  const input = document.getElementById("guard-group-name") as HTMLInputElement | null;
  if (title) title.textContent = t("Rename group");
  if (submit) submit.textContent = t("Save");
  if (input) input.value = group.name;
  showGuardModal("guard-group-modal");
}

export function closeGuardGroupModal(): void {
  hideGuardModal("guard-group-modal");
  guardGroupFormId = null;
}

export async function saveGuardGroupForm(): Promise<void> {
  if (guardWriteBlocked()) return;
  const input = document.getElementById("guard-group-name") as HTMLInputElement | null;
  const name = input?.value.trim() ?? "";
  if (!name) {
    toast(t("Please enter a group name"), "error");
    input?.focus();
    return;
  }
  try {
    if (guardGroupFormMode === "rename" && guardGroupFormId) {
      await contractGroupRename(guardGroupFormId, name);
      toast(t("Group renamed"), "success");
    } else {
      await contractGroupCreate(name);
      toast(t("Group created"), "success");
    }
    closeGuardGroupModal();
    await refreshGuardView(true);
  } catch (error) {
    toast(t("Group action failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
}

export async function guardGroupDelete(id: string): Promise<void> {
  if (guardWriteBlocked()) return;
  const group = guardUiState.view?.groups.find((candidate) => candidate.id === id);
  if (!group || group.builtin) return;
  if (!(await ask(t("Delete group {{name}}? It must be empty first.", { name: group.name }), { title: t("Delete group"), kind: "warning" }))) return;
  try {
    await contractGroupDelete(id);
    toast(t("Group deleted"), "success");
    await refreshGuardView(true);
  } catch (error) {
    toast(t("Group action failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
}

export async function guardGroupMove(id: string, direction: "up" | "down"): Promise<void> {
  if (guardWriteBlocked()) return;
  const groups = (guardUiState.view?.groups ?? []).filter((group) => !group.builtin);
  const index = groups.findIndex((group) => group.id === id);
  if (index < 0) return;
  const target = direction === "up" ? index - 1 : index + 1;
  if (target < 0 || target >= groups.length) return;
  const ids = groups.map((group) => group.id);
  [ids[index], ids[target]] = [ids[target], ids[index]];
  try {
    await contractGroupReorder(ids);
    await refreshGuardView(true);
  } catch (error) {
    toast(t("Group action failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
}

export async function guardParameterMove(id: string, groupId: string): Promise<void> {
  if (guardWriteBlocked()) return;
  const parameter = guardUiState.view?.groups.flatMap((group) => group.params).find((candidate) => candidate.id === id);
  if (!parameter || parameter.groupId === groupId) return;
  try {
    await contractParameterMove(id, groupId);
    toast(t("Parameter moved"), "success");
    await refreshGuardView(true);
  } catch (error) {
    toast(t("Parameter move failed: {{error}}", { error: guardErrorLabel(error) }), "error");
    await refreshGuardView(true);
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
  const groups = view.groups.filter((group) => !(group.id === "uncategorized" && group.params.length === 0));
  const groupCards = groups.map((g) => {
    const params = g.params.map((p) => {
      const s = statusMap[p.status] ?? statusMap.error;
      let editor = "";
      const dis = p.locked || guardWriteBlocked() ? "disabled" : "";
      if (p.valueType === "bool") {
        editor = `<div class="flex items-center gap-2">
          <input id="guard-param-input-${escapeHtml(p.id)}" aria-label="${escapeHtml(p.label)}" type="checkbox" class="relative h-5 w-9 shrink-0 cursor-pointer appearance-none rounded-full bg-input transition-colors outline-none checked:bg-primary focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 before:absolute before:left-0.5 before:top-0.5 before:h-4 before:w-4 before:rounded-full before:bg-background before:shadow-sm before:transition-transform checked:before:translate-x-4" data-change-action="guardToggleBool" data-id="${p.id}"
                 ${p.value === true ? "checked" : ""} ${p.locked ? "disabled" : ""} />
          <span class="text-xs opacity-70">${p.value === true ? "true" : "false"} ${t("(recommended {{default}})", { default: String(p.default) })}</span>
        </div>`;
      } else if (p.valueType === "int" || p.valueType === "string") {
        const t = p.valueType === "int" ? "number" : "text";
        editor = `<input id="guard-param-input-${escapeHtml(p.id)}" aria-label="${escapeHtml(p.label)}" type="${t}" class="h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-xs outline-none transition-colors placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 font-mono" ${dis}
               value="${escapeHtml(String(p.value ?? ""))}" data-guard-id="${p.id}"
               data-change-action="guardSetValue" data-id="${p.id}" />`;
      } else if (p.valueType === "text") {
        editor = `<textarea id="guard-param-input-${escapeHtml(p.id)}" aria-label="${escapeHtml(p.label)}" class="guard-textarea" ${dis} data-guard-id="${p.id}"
               data-change-action="guardSetValue" data-id="${p.id}">${escapeHtml(String(p.value ?? ""))}</textarea>`;
      } else {
        editor = `<span class="text-xs opacity-60">${t("No editable value; applying performs \"{{action}}\"", { action: t(p.applyMode === "toml_absent" ? "delete" : "write") })}</span>`;
      }
      const meta = p.locked
        ? `<div class="mt-1 text-xs opacity-50">${t("Last checked {{checked}} | Last auto-restored {{restored}}", { checked: fmtTs(p.lastChecked), restored: fmtTs(p.lastRestored) })}</div>`
        : "";
      const movableGroups = view.groups.filter((candidate) => !candidate.builtin && candidate.id !== g.id);
      const moveControl = p.custom && movableGroups.length > 0
        ? `<label class="guard-param-move"><span class="sr-only">${escapeHtml(t("Move parameter"))}</span><select data-change-action="guardParameterMove" data-id="${escapeHtml(p.id)}" aria-label="${escapeHtml(t("Move parameter"))}"><option value="${escapeHtml(g.id)}">${escapeHtml(t("Current group"))}</option>${movableGroups.map((candidate) => `<option value="${escapeHtml(candidate.id)}">${escapeHtml(candidate.name)}</option>`).join("")}</select></label>`
        : "";
      return `<div class="guard-param-card rounded-lg border border-border bg-card text-card-foreground p-3">
        <div class="flex items-center justify-between">
          <label class="text-sm font-medium" for="guard-param-input-${escapeHtml(p.id)}">${escapeHtml(p.label)}${p.description || p.path ? ` <span class="guard-param-help" tabindex="0">?<span class="guard-param-desc">${p.description ? `<span>${escapeHtml(p.description)}</span>` : ""}${p.path ? `<span class="guard-param-desc-path">${escapeHtml(p.path)}</span>` : ""}</span></span>` : ""}</label>
          <span class="status-badge ${s.cls}"><span class="dot"></span><span>${s.text}</span></span>
        </div>
        <div class="mt-1 flex items-start justify-between gap-2">
          <div class="min-w-0 flex-1">
            <div class="guard-param-actual font-mono text-xs ${p.status === "match" ? "ok" : "bad"}">
              ${t("Current: ")}${escapeHtml(p.actual ?? p.error ?? t("Unknown"))}
            </div>
            <div class="mt-2">${editor}</div>
            ${meta}
          </div>
          <span class="guard-param-actions flex w-[30%] shrink-0 flex-row flex-wrap items-center justify-end gap-1 self-center">
            ${moveControl}
            <input type="checkbox" class="text-switch" data-change-action="guardToggleApplied" data-id="${p.id}" data-state-text="${p.applied ? t("Enabled") : t("Disabled")}"
                   ${p.applied ? "checked" : ""} ${p.locked ? "disabled" : ""} title="${p.applied ? t("Disable") : t("Enable")}" aria-label="${p.applied ? t("Disable") : t("Enable")}" />
            ${p.locked
              ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-1 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="guardSetLocked" data-id="${p.id}" data-locked="false">${LOCK_SVG}${t("Unlock")}</button>`
              : `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-1 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" ${p.applied && !guardWriteBlocked() ? "" : "disabled"}
                    data-action="guardSetLocked" data-id="${p.id}" data-locked="true">${UNLOCK_SVG}${t("Lock")}</button>`}
            ${p.custom ? `<button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-destructive/50 bg-background text-destructive hover:bg-destructive/10 h-6 px-2 text-xs" data-action="guardRemoveCustom" data-id="${p.id}" title="${t("Delete custom parameter")}" ${guardWriteBlocked() ? "disabled" : ""}>${t("Delete")}</button>` : ""}
          </span>
        </div>
      </div>`;
    }).join("");
    const addBtn = `<div class="mt-2">
      <button class="inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-6 px-2 text-xs" data-action="openGuardAddFormFor" data-id="${g.id}">${t("+ Add Parameter")}</button>
    </div>`;
    return `<div class="guard-group-card rounded-xl border border-border bg-card text-card-foreground p-4" data-group-id="${g.id}">
      ${renderGroupHeader(g)}
      <div class="guard-legacy-group-meta text-sm font-semibold">${escapeHtml(g.name)}</div>
      <div class="guard-legacy-group-meta mb-2 font-mono text-xs opacity-50">~/.codex/${escapeHtml(g.file)}</div>
      ${g.error ? `<div class="guard-group-diagnostic">${escapeHtml(t("Guard group has a diagnostic"))}</div>` : ""}
      <div class="flex flex-col gap-2">${params}</div>
      ${addBtn}
    </div>`;
  }).join("");
  container.innerHTML = `<div class="guard-group-toolbar"><span class="guard-section-heading">${escapeHtml(t("Parameter groups"))}</span><button class="guard-role-button" data-action="guardGroupCreate" ${guardWriteBlocked() ? "disabled" : ""}>${escapeHtml(t("Create group"))}</button></div>${groupCards}`;
}

const ACTION_ICON: Record<BatchAction, string> = {
  apply: `<svg data-icon="CircleCheck" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="m8 12 2.5 2.5L16 9"/></svg>`,
  lock: `<svg data-icon="LockKeyhole" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="16" height="11" x="4" y="10" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/></svg>`,
  unlock: `<svg data-icon="UnlockKeyhole" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="16" height="11" x="4" y="10" rx="2"/><path d="M8 10V7a4 4 0 0 1 7.7-1.5"/></svg>`,
  disable: `<svg data-icon="CircleOff" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="m6 6 12 12"/></svg>`,
};

const ACTION_LABEL: Record<BatchAction, string> = {
  apply: "Apply",
  lock: "Lock",
  unlock: "Unlock",
  disable: "Disable",
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null ? value as Record<string, unknown> : null;
}

function axisValue(value: unknown, key: string): string {
  const record = asRecord(value);
  if (typeof record?.[key] === "string") return record[key] as string;
  const params = Array.isArray(record?.params) ? record.params : Array.isArray(record?.groups)
    ? record.groups.flatMap((group) => {
      const groupRecord = asRecord(group);
      return Array.isArray(groupRecord?.params) ? groupRecord.params : [];
    })
    : [];
  if (params.length === 0) return "unknown";
  if (key === "lifecycle" || key === "lifecycleSummary") {
    const values = params.map((param) => {
      const item = asRecord(param);
      return item?.locked === true ? "locked" : item?.applied === true ? "applied" : item?.applied === false ? "disabled" : "unknown";
    });
    if (values.includes("unknown")) return "unknown";
    return values.every((item) => item === values[0]) ? values[0] : "mixed";
  }
  if (key === "health" || key === "healthSummary") {
    const values = params.map((param) => {
      const status = asRecord(param)?.status;
      if (status === "match") return "healthy";
      if (status === "drift") return "drifted";
      if (status === "missing") return "drifted";
      if (status === "error") return "error";
      return "unknown";
    });
    if (values.includes("error")) return "error";
    if (values.includes("unknown")) return "unknown";
    if (values.includes("drifted")) return "drifted";
    return "healthy";
  }
  return "unknown";
}

function ownerMemberCount(owner: unknown): number | null {
  const record = asRecord(owner);
  if (Array.isArray(record?.params)) return record.params.length;
  if (Array.isArray(record?.groups)) {
    return record.groups.reduce((count, group) => {
      const params = asRecord(group)?.params;
      return count + (Array.isArray(params) ? params.length : 0);
    }, 0);
  }
  return null;
}

function actionState(owner: unknown, action: BatchAction, scope?: BatchScope): GuardActionState {
  if (owner === null && !guardUiState.view) return { enabled: false, reason: "state_unavailable" };
  if (guardUiState.batch?.busy) return { enabled: false, reason: "busy" };
  if (guardUiState.recovery?.blocked) return { enabled: false, reason: "recovery_in_progress" };
  const record = asRecord(owner);
  const actions = asRecord(record?.actions) ?? asRecord(record?.actionStates);
  const candidate = asRecord(actions?.[action]);
  if (candidate && typeof candidate.enabled === "boolean") {
    return { enabled: candidate.enabled, reason: typeof candidate.reason === "string" ? candidate.reason : null };
  }
  if (scope && scope !== "all" && "role" in scope) {
    const role = asRecord(owner);
    const lifecycle = axisValue(owner, "lifecycle");
    const health = axisValue(owner, "health");
    if (ownerMemberCount(owner) === 0) return { enabled: false, reason: "state_unavailable" };
    if ((action === "apply" || action === "lock") && ["invalid", "unsupported", "error", "unknown"].includes(health)) {
      return { enabled: false, reason: health === "unknown" ? "state_unavailable" : health };
    }
    // Role scopes are expanded by the backend. Lifecycle/health remain DTO-owned.
    if (role && typeof role.id === "string" && lifecycle === "unknown") return { enabled: false, reason: "state_unavailable" };
    return { enabled: true, reason: null };
  }
  if (ownerMemberCount(owner) === 0) return { enabled: false, reason: "state_unavailable" };
  const health = axisValue(owner, "health");
  if (action === "apply" || action === "lock") {
    if (health === "invalid") return { enabled: false, reason: "invalid" };
    if (health === "unsupported") return { enabled: false, reason: "unsupported" };
    if (health === "error") return { enabled: false, reason: "error" };
    if (health === "unknown") return { enabled: false, reason: "state_unavailable" };
  }
  // The backend remains authoritative; this projection keeps obvious invalid states disabled
  // when older GuardView DTOs omit explicit action-state fields.
  return { enabled: true, reason: null };
}

function axisLabel(value: string, kind: "lifecycle" | "health"): string {
  const labels: Record<string, string> = kind === "lifecycle"
    ? { disabled: "Disabled", applied: "Applied", locked: "Locked", mixed: "Mixed" }
    : { healthy: "Healthy", drifted: "Drifted", invalid: "Invalid", unsupported: "Unsupported", error: "Error" };
  return t(labels[value] ?? "Unknown");
}

function toneClass(value: unknown): string {
  return `guard-tone-${statusTone(value)}`;
}

function actionReason(reason: string | null): string {
  if (!reason) return "";
  const labels: Record<string, string> = {
    busy: "Operation in progress",
    recovery_in_progress: "Recovery is blocking Guard writes",
    invalid: "Invalid input",
    unsupported: "Unsupported action",
    error: "Guard state has an error",
    state_unavailable: "Guard state is unavailable",
    ownership_uncertain: "File ownership is uncertain",
    role_scope_unavailable: "Role actions are unavailable until role membership is connected",
  };
  return t(labels[reason] ?? "Operation unavailable");
}

function actionButton(scope: BatchScope, action: BatchAction, owner: unknown): string {
  const stateForAction = actionState(owner, action, scope);
  const label = t(ACTION_LABEL[action]);
  const disabled = stateForAction.enabled ? "" : "disabled";
  const reason = actionReason(stateForAction.reason);
  const scopeValue = scope === "all" ? "all" : JSON.stringify(scope);
  const ownerRecord = asRecord(owner);
  const scopeLabel = scope === "all" ? t("Global") : (typeof ownerRecord?.name === "string" ? ownerRecord.name : typeof ownerRecord?.displayName === "string" ? ownerRecord.displayName : t("Group"));
  return `<button class="guard-action-button ${action === "disable" ? "is-danger" : ""}" data-action="guardBatch" data-scope='${escapeHtml(scopeValue)}' data-batch-action="${action}" ${disabled} title="${escapeHtml(reason || label)}" aria-label="${escapeHtml(`${label} ${scopeLabel}`)}" ${reason ? `data-reason="${escapeHtml(reason)}"` : ""}>${ACTION_ICON[action]}<span class="guard-action-label">${escapeHtml(label)}</span></button>`;
}

function actionReasons(owner: unknown, scope?: BatchScope): string {
  return actionOrder.map((action) => {
    const reason = actionReason(actionState(owner, action, scope).reason);
    return reason ? `${t(ACTION_LABEL[action])}: ${reason}` : "";
  }).filter(Boolean).join(" · ");
}

function renderGroupHeader(group: GuardView["groups"][number]): string {
  const scope: BatchScope = { group: { groupId: group.id } };
  const files = Array.isArray(group.files) && group.files.length > 0 ? group.files : [{ file: group.file, format: group.format }];
  const reasons = actionReasons(group, scope);
  const custom = !group.builtin;
  const groupIndex = custom ? (guardUiState.view?.groups.filter((candidate) => !candidate.builtin).findIndex((candidate) => candidate.id === group.id) ?? -1) : -1;
  const customCount = guardUiState.view?.groups.filter((candidate) => !candidate.builtin).length ?? 0;
  const groupControls = custom
    ? `<div class="guard-group-controls"><button class="guard-role-button" data-action="guardGroupMoveUp" data-id="${escapeHtml(group.id)}" ${groupIndex <= 0 || guardWriteBlocked() ? "disabled" : ""} title="${escapeHtml(t("Move group up"))}">↑</button><button class="guard-role-button" data-action="guardGroupMoveDown" data-id="${escapeHtml(group.id)}" ${groupIndex < 0 || groupIndex >= customCount - 1 || guardWriteBlocked() ? "disabled" : ""} title="${escapeHtml(t("Move group down"))}">↓</button><button class="guard-role-button" data-action="guardGroupRename" data-id="${escapeHtml(group.id)}" ${guardWriteBlocked() ? "disabled" : ""}>${escapeHtml(t("Rename"))}</button><button class="guard-role-button is-danger" data-action="guardGroupDelete" data-id="${escapeHtml(group.id)}" ${guardWriteBlocked() ? "disabled" : ""}>${escapeHtml(t("Delete"))}</button></div>`
    : "";
  return `<div class="guard-group-header"><div class="guard-group-title"><span class="text-sm font-semibold">${escapeHtml(group.name)}</span><span class="guard-group-files">${files.map((file) => `~/.codex/${escapeHtml(file.file)} (${escapeHtml(file.format)})`).join(" · ")}</span><div class="guard-axis-grid compact"><div class="guard-axis"><span class="guard-axis-label">${escapeHtml(t("Lifecycle"))}</span><strong class="${toneClass(axisValue(group, "lifecycle"))}">${escapeHtml(axisLabel(axisValue(group, "lifecycle"), "lifecycle"))}</strong></div><div class="guard-axis"><span class="guard-axis-label">${escapeHtml(t("Health"))}</span><strong class="${toneClass(axisValue(group, "health"))}">${escapeHtml(axisLabel(axisValue(group, "health"), "health"))}</strong></div></div></div><div class="guard-group-header-actions">${groupControls}<div class="guard-action-bar">${actionOrder.map((action) => actionButton(scope, action, group)).join("")}</div></div>${reasons ? `<div class="guard-action-reasons">${escapeHtml(reasons)}</div>` : ""}</div>`;
}

function renderGuardSummary(view: GuardView): void {
  const summary = document.getElementById("guard-global-summary");
  if (!summary) return;
  const lifecycle = axisValue(view, "lifecycle") !== "unknown" ? axisValue(view, "lifecycle") : axisValue(view, "lifecycleSummary");
  const health = axisValue(view, "health") !== "unknown" ? axisValue(view, "health") : axisValue(view, "healthSummary");
  summary.innerHTML = `<div class="guard-summary-heading"><span>${escapeHtml(t("Global summary"))}</span><span class="guard-summary-enabled ${view.enabled ? "is-on" : "is-off"}">${escapeHtml(view.enabled ? t("Enabled") : t("Disabled"))}</span></div><div class="guard-axis-grid"><div class="guard-axis"><span class="guard-axis-label">${escapeHtml(t("Runtime state"))}</span><strong class="${toneClass(view.runtimeState)}">${escapeHtml(view.runtimeState === "running" ? t("Running") : t("Suspended"))}</strong></div><div class="guard-axis"><span class="guard-axis-label">${escapeHtml(t("Lifecycle"))}</span><strong class="${toneClass(lifecycle)}">${escapeHtml(axisLabel(lifecycle, "lifecycle"))}</strong></div><div class="guard-axis"><span class="guard-axis-label">${escapeHtml(t("Health"))}</span><strong class="${toneClass(health)}">${escapeHtml(axisLabel(health, "health"))}</strong></div><div class="guard-axis"><span class="guard-axis-label">${escapeHtml(t("Members / files"))}</span><strong>${escapeHtml(`${view.affectedMembers} / ${view.affectedFiles}`)}</strong></div></div>`;
}

function renderGuardDiagnostics(): void {
  const container = document.getElementById("guard-diagnostics");
  if (!container) return;
  const diagnostics = [...(guardUiState.batch?.preview?.diagnostics ?? []), ...(guardUiState.batch?.report?.diagnostics ?? [])];
  const errors = [guardUiState.viewError, guardUiState.filesError, guardUiState.recoveryError, guardUiState.batch?.error].filter(Boolean) as string[];
  if (diagnostics.length === 0 && errors.length === 0) {
    container.innerHTML = `<span class="guard-empty-diagnostic">${escapeHtml(t("No diagnostics"))}</span>`;
    return;
  }
  const diagnosticHtml = diagnostics.map((diagnostic) => {
    const info = diagnosticMessage(diagnostic);
    const location = [info.file, info.field, info.line === null ? null : `L${info.line}`].filter(Boolean).join(" · ");
    const copyValue = [info.file, info.field, info.line === null ? null : `L${info.line}`].filter(Boolean).join(" · ");
    return `<li><span class="guard-diagnostic-code">${escapeHtml(diagnosticLabel(info.code))}</span>${location ? `<span class="guard-diagnostic-location guard-copyable" tabindex="0" role="button" data-copy-value="${escapeHtml(copyValue)}" title="${escapeHtml(t("Copy to clipboard"))}">${escapeHtml(location)}</span>` : ""}</li>`;
  }).join("");
  container.innerHTML = `${errors.map((error) => `<div class="guard-diagnostic-error">${escapeHtml(error === guardUiState.viewError ? t("Guard view unavailable") : error === guardUiState.filesError ? t("Guard files unavailable") : error === guardUiState.recoveryError ? t("Recovery status unavailable") : guardErrorLabel(error))}</div>`).join("")}${diagnosticHtml ? `<ul>${diagnosticHtml}</ul>` : ""}`;
}

function renderGuardOperation(): void {
  const container = document.getElementById("guard-operation-region");
  if (!container) return;
  const batch = guardUiState.batch;
  if (!batch) {
    container.innerHTML = `<span class="guard-empty-diagnostic">${escapeHtml(t("No operation in progress"))}</span>`;
    container.removeAttribute("aria-busy");
    return;
  }
  const report = batch.report;
  const reportText = report
    ? report.outcome === "committed"
      ? t("Committed: {{changed}} changed, {{unchanged}} unchanged, {{files}} files", { changed: report.changed, unchanged: report.unchanged, files: report.files })
      : report.outcome === "rolled_back"
        ? t("Rolled back: changes were restored")
        : report.outcome === "critical_recovery"
          ? t("Critical recovery required")
          : t("Operation rejected")
    : batch.error
      ? (batch.error === "batch_preview_stale" ? t("Preview expired; confirm again") : t("Operation failed"))
      : batch.preview
        ? t("Preview: {{members}} members, {{files}} files", { members: batch.preview.memberIds.length, files: batch.preview.files })
        : t("Preparing operation");
  const tone = report ? statusTone(report.outcome) : batch.error ? "negative" : "warning";
  container.setAttribute("role", report?.outcome === "critical_recovery" ? "alert" : "status");
  container.setAttribute("aria-busy", String(batch.busy));
  const phases = operationPhases.map((phase) => `<li class="guard-operation-phase ${batch.phase === phase ? "is-current" : operationPhases.indexOf(phase) < operationPhases.indexOf(batch.phase) ? "is-done" : ""}"><span class="guard-operation-phase-dot" aria-hidden="true"></span><span>${escapeHtml(operationPhaseLabel(phase))}</span></li>`).join("");
  container.innerHTML = `<div class="guard-operation-status ${toneClass(tone)}"><span>${escapeHtml(reportText)}</span>${batch.busy ? `<span class="guard-operation-spinner" aria-hidden="true"></span>` : ""}</div><ol class="guard-operation-phases" aria-label="${escapeHtml(t("Operation phases"))}">${phases}</ol><div class="guard-operation-progress"><span style="width:${Math.round(batch.progress * 100)}%"></span></div>`;
}

function renderGuardRecovery(): void {
  const container = document.getElementById("guard-recovery-region");
  if (!container) return;
  const recovery = guardUiState.recovery;
  if (!recovery?.blocked) {
    container.classList.add("hidden");
    container.removeAttribute("role");
    container.innerHTML = "";
    return;
  }
  container.classList.remove("hidden");
  container.setAttribute("role", "alert");
  const recoveryRecord = asRecord(recovery);
  const unrecoveredFiles = Array.isArray(recoveryRecord?.unrecoveredFiles)
    ? recoveryRecord.unrecoveredFiles.filter((file): file is string => typeof file === "string")
    : [];
  const filesHtml = unrecoveredFiles.length > 0
    ? `<div class="guard-recovery-files"><span>${escapeHtml(t("Unrecovered files"))}</span><ul>${unrecoveredFiles.map((file) => `<li>${escapeHtml(file)}</li>`).join("")}</ul></div>`
    : "";
  container.innerHTML = `<div class="guard-recovery-title">${escapeHtml(t("Critical recovery"))}</div><div>${escapeHtml(t("Guard writes are paused until the pending transaction is recovered."))}</div><div class="guard-recovery-code guard-copyable" tabindex="0" role="button" title="${escapeHtml(t("Copy to clipboard"))}" data-copy-value="${escapeHtml(recovery.code ?? "")}">${escapeHtml(recovery.code ? t("Recovery code: {{code}}", { code: recovery.code }) : t("Recovery status unavailable"))}</div>${filesHtml}<div class="guard-recovery-actions"><button class="guard-recovery-retry" data-action="guardRetryRecovery">${escapeHtml(t("Retry recovery"))}</button><button class="guard-recovery-open" data-action="guardOpenBackupDir">${escapeHtml(t("Open backup directory"))}</button></div>`;
}

function renderGuardGlobalActions(): void {
  const container = document.getElementById("guard-global-actions");
  if (!container) return;
  container.querySelectorAll<HTMLButtonElement>('[data-action="guardBatch"]').forEach((button) => {
    const action = button.dataset.batchAction as BatchAction | undefined;
    if (!action) return;
    const stateForAction = actionState(guardUiState.view, action);
    button.disabled = !stateForAction.enabled;
    const reason = actionReason(stateForAction.reason);
    button.title = reason || t(ACTION_LABEL[action]);
    button.setAttribute("aria-label", `${t(ACTION_LABEL[action])} ${t("Global")}`);
    if (reason) button.dataset.reason = reason;
    else delete button.dataset.reason;
  });
  const reasonEl = container.querySelector<HTMLElement>('[data-action-reasons]');
  if (reasonEl) {
    const reasons = actionReasons(guardUiState.view);
    reasonEl.textContent = reasons;
    reasonEl.classList.toggle("hidden", !reasons);
  }
}

function renderGuardWorkbench(renderView = true): void {
  const root = document.getElementById("guard-view");
  if (root) root.setAttribute("aria-busy", String(guardUiState.batch?.busy ?? false));
  if (renderView && guardUiState.view) {
    renderGuardSummary(guardUiState.view);
    renderGuardView(guardUiState.view);
  }
  renderGuardGlobalActions();
  renderGuardDiagnostics();
  renderGuardOperation();
  renderGuardRecovery();
  renderGuardRoleWorkbench();
}

function batchRequest(scope: BatchScope, action: BatchAction): BatchRequest {
  return { schemaVersion: GUARD_CONTRACT.schemaVersion, scope, action };
}

function batchConfirmation(preview: { memberIds: string[]; files: number }, action: BatchAction): string {
  return t("Confirm {{action}} for {{members}} members across {{files}} files?", {
    action: t(ACTION_LABEL[action]),
    members: preview.memberIds.length,
    files: preview.files,
  });
}

function batchOutcomeToast(outcome: string): void {
  if (outcome === "committed") toast(t("Committed"), "success");
  else if (outcome === "rolled_back") toast(t("Rolled back: changes were restored"), "error");
  else if (outcome === "critical_recovery") toast(t("Critical recovery required"), "error");
  else toast(t("Operation rejected"), "error");
}

/**
 * 取出后端返回的稳定 error code。Guard 命令的错误经 typed wrapper 包成
 * `GuardContractError`，`String(err)` 会带上 `GuardContractError: ` 前缀——
 * 只剥 `Error:` 会让所有 code 匹配失败，把每个错误都显示成通用文案。
 */
export function guardErrorCode(error: unknown): string {
  if (error instanceof Error) return error.message.trim();
  return String(error)
    .replace(/^[A-Za-z]*Error:\s*/, "")
    .trim();
}

function guardErrorLabel(error: unknown): string {
  const code = guardErrorCode(error);
  const labels: Record<string, string> = {
    batch_preview_required: "Preview is required",
    batch_preview_stale: "Preview expired; confirm again",
    batch_scope_empty: "No eligible members",
    guard_contract_version_unsupported: "Guard contract is unsupported",
    recovery_failed: "Recovery failed",
    recovery_in_progress: "Recovery is blocking Guard writes",
    guard_busy: "Another Guard operation is running",
    recovery_blocked: "Recovery is blocking Guard writes",
    migration_failed: "Migration failed",
    format_migration_pending: "Choose a file format before writing",
    lifecycle_migration_pending: "Resolve the pending lifecycle before writing",
  };
  return t(labels[code] ?? "Operation unavailable");
}

function dispatchLocalProgress(operationId: string, phase: GuardOperationPhase, progress: number): void {
  dispatchGuard({ type: "batch/progress", operationId, phase, progress });
  renderGuardWorkbench(false);
}

/**
 * Accepts backend progress events. The backend batch id is minted inside the transaction
 * (`{pid}-{nanos}-{serial}`) and can never equal the frontend operation id, so events are
 * bound to whichever operation is currently in flight and then pinned to that batch id.
 */
export function handleGuardProgressEvent(payload: unknown): void {
  const record = asRecord(payload);
  const batch = guardUiState.batch;
  if (!batch || !batch.busy) return;
  const batchId = typeof record?.batchId === "string" ? record.batchId : null;
  if (!batchId) return;
  // First event of an in-flight operation adopts the backend id; later events from a
  // different batch are stale and must not move this operation's progress.
  if (batch.backendBatchId && batch.backendBatchId !== batchId) return;
  const operationId = batch.operationId;
  const phase = normalizeOperationPhase(record?.phase);
  const completed = typeof record?.completed === "number" ? record.completed : undefined;
  const total = typeof record?.total === "number" && record.total > 0 ? record.total : undefined;
  dispatchGuard({
    type: "batch/progress",
    operationId,
    batchId,
    phase,
    progress: completed !== undefined && total ? normalizeProgress(completed / total) : normalizeProgress(record?.progress),
  });
  renderGuardWorkbench(false);
}

export async function copyGuardValue(value: string): Promise<void> {
  if (!value) return;
  try {
    await navigator.clipboard.writeText(value);
    toast(t("Copied to clipboard"), "info");
  } catch (error) {
    toast(t("Copy failed: {{error}}", { error: String(error) }), "error");
  }
}

/** Execute one backend batch; Apply/Lock preview silently for group scopes. */
export async function guardBatch(scope: BatchScope, action: BatchAction): Promise<void> {
  if (guardUiState.batch?.busy) return;
  const request = batchRequest(scope, action);
  const operationId = nextOperationId();
  dispatchGuard({ type: "batch/started", operationId, request });
  renderGuardWorkbench();
  try {
    let previewId: string | null = null;
    if (action === "apply" || action === "lock") {
      dispatchLocalProgress(operationId, "preflight", 0.12);
      const preview = await contractPreviewBatch(request);
      dispatchGuard({ type: "batch/preview", operationId, preview });
      renderGuardWorkbench();
      if (!preview.eligible) {
        dispatchGuard({ type: "batch/error", operationId, error: "batch_preview_rejected" });
        renderGuardWorkbench();
        return;
      }
      previewId = preview.previewId;
      if (shouldConfirmBatch(scope, action) && !(await ask(batchConfirmation(preview, action), { title: t("Confirm Guard operation"), kind: "warning" }))) {
        dispatchGuard({ type: "batch/error", operationId, error: "batch_cancelled" });
        renderGuardWorkbench();
        return;
      }
    }
    // 不在 await 之前伪造 snapshot/write：那两个阶段由后端事务决定，
    // 同步派发只会让进度条停在一个从未发生的阶段。真实阶段由 guard-operation-progress 驱动。
    const report = await contractExecuteBatch(request, previewId);
    dispatchLocalProgress(operationId, "verify", 0.82);
    dispatchGuard({ type: "batch/report", operationId, report });
    batchOutcomeToast(report.outcome);
  } catch (error) {
    dispatchGuard({ type: "batch/error", operationId, error: guardErrorCode(error) });
    toast(t("Operation failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
  renderGuardWorkbench();
  await refreshGuardView(true);
  await refreshGuardOperationAudit();
}

export async function refreshGuardRecovery(): Promise<void> {
  try {
    const recovery = await contractGetRecoveryStatus();
    dispatchGuard({ type: "recovery/success", recovery });
  } catch (error) {
    dispatchGuard({ type: "recovery/error", error: guardErrorCode(error) });
  }
  renderGuardWorkbench();
}

export async function guardOpenBackupDir(): Promise<void> {
  try {
    const { homeDir, join } = await import("@tauri-apps/api/path");
    // homeDir() has no trailing separator, so string concatenation would yield
    // `/Users/namedashi-backups` and the only manual recovery route would always fail.
    await openUrl(await join(await homeDir(), ".codex", "dashi-backups"));
  } catch (error) {
    toast(t("Open backup directory failed: {{error}}", { error: String(error) }), "error");
  }
}

export async function guardRetryRecovery(): Promise<void> {
  try {
    await contractRetryRecovery();
    toast(t("Recovery completed"), "success");
    await refreshGuardRecovery();
    await refreshGuardView(true);
  } catch (error) {
    dispatchGuard({ type: "recovery/error", error: guardErrorCode(error) });
    renderGuardWorkbench();
    toast(t("Recovery failed: {{error}}", { error: guardErrorLabel(error) }), "error");
  }
}

export async function guardToggleBool(id: string): Promise<void> {
  if (guardWriteBlocked()) return;
  const st = state.guardState.params[id];
  if (st?.locked) return;
  try {
    const view = await contractGetView();
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    await contractSetValue(id, p.value !== true);
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Change failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function guardSetValue(id: string, input: HTMLInputElement | HTMLTextAreaElement): Promise<void> {
  if (guardWriteBlocked()) return;
  try {
    const view = await contractGetView();
    const p = view.groups.flatMap((g) => g.params).find((x) => x.id === id);
    if (!p) return;
    const value = p.valueType === "int" ? parseInt(input.value, 10) : input.value;
    if (p.valueType === "int" && Number.isNaN(value)) {
      toast(t("Please enter an integer"), "error");
      await refreshGuardView(true);
      return;
    }
    await contractSetValue(id, value as JsonValue);
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Save failed: {{error}}", { error: String(e) }), "error");
    await refreshGuardView(true);
  }
}

export async function guardApply(id: string): Promise<void> {
  await guardBatch({ parameter: { parameterId: id } }, "apply");
}

export async function guardToggleApplied(id: string): Promise<void> {
  if (guardWriteBlocked()) return;
  try {
    const view = await contractGetView();
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
  await guardBatch({ parameter: { parameterId: id } }, "disable");
}

export async function guardSetLocked(id: string, locked: boolean): Promise<void> {
  await guardBatch({ parameter: { parameterId: id } }, locked ? "lock" : "unlock");
}

// ============ 文件管理 ============
let guardFiles: GuardFileView[] = [];
let guardAddParamFileId: string | null = null;

export async function refreshGuardFiles(): Promise<void> {
  try {
    guardFiles = await contractGetFiles();
    dispatchGuard({ type: "files/success", files: guardFiles });
    renderGuardFiles();
    // 首次（无检测记录）自动检测一次并落盘；之后直接读记录，不重复扫盘
    for (const f of guardFiles) {
      if (f.builtin && !f.detection) {
        await guardDetectFile(f.id, true);
      }
    }
  } catch (e) {
    dispatchGuard({ type: "files/error", error: String(e) });
    renderGuardWorkbench();
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
  if (modal?.classList.contains("hidden")) showGuardModal("guard-file-modal");
  else hideGuardModal("guard-file-modal");
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
  showGuardModal("guard-file-modal");
}

export async function guardPickFilePath(): Promise<void> {
  try {
    const selected = await openDialog({ multiple: false });
    if (typeof selected !== "string") return;
    const rel = await contractRelativizePickedPath(selected);
    (document.getElementById("settings-guard-file-path") as HTMLInputElement).value = rel;
    // 顺手带入文件名与格式
    const nameEl = document.getElementById("settings-guard-file-name") as HTMLInputElement;
    const fileName = rel.split("/").pop() ?? rel;
    if (!nameEl.value.trim()) nameEl.value = fileName;
    const ext = fileName.split(".").pop()?.toLowerCase();
    const formatByExtension: Partial<Record<string, GuardFileFormat>> = {
      toml: "toml",
      json: "json",
      md: "markdown",
      txt: "plain_text",
    };
    if (ext && formatByExtension[ext]) {
      (document.getElementById("settings-guard-file-format") as HTMLSelectElement).value = formatByExtension[ext];
    }
  } catch (e) {
    toast(`${e}`, "error");
  }
}

export async function guardSaveFileForm(): Promise<void> {
  if (guardWriteBlocked()) return;
  const name = (document.getElementById("settings-guard-file-name") as HTMLInputElement).value.trim();
  const file = (document.getElementById("settings-guard-file-path") as HTMLInputElement).value.trim();
  const format = (document.getElementById("settings-guard-file-format") as HTMLSelectElement).value as GuardFileFormat;
  if (!name) { toast(t("Please enter a file name"), "error"); return; }
  if (!file) { toast(t("Please enter a file path"), "error"); return; }
  try {
    if (editingFileId) {
      await contractUpdateFile(editingFileId, name, file);
      toast(t("Updated"), "success");
    } else {
      await contractAddFile(name, file, format);
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
    const updated = await contractDetectFile(id);
    guardFiles = guardFiles.map((x) => (x.id === id ? updated : x));
    dispatchGuard({ type: "files/success", files: guardFiles });
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
        await contractUpdateFile(id, updated.name, detected);
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
  if (guardWriteBlocked()) return;
  const f = guardFiles.find((x) => x.id === id);
  if (!f) return;
  if (!(await ask(t("Delete file \"{{name}}\"?\n\nAll custom parameters under it will be unguarded, but values already written to ~/.codex/{{file}} will not be rolled back.", { name: f.name, file: f.file }), { title: t("Delete Guard File"), kind: "warning" }))) {
    return;
  }
  try {
    await contractRemoveFile(id);
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
  showGuardModal("guard-add-modal");
  onGuardAddModeChange();
  onGuardAddValueTypeChange();
}

export function closeGuardAddModal(): void {
  hideGuardModal("guard-add-modal");
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

function parseDefaultValue(value: string, effectiveType: string): JsonValue {
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
  if (guardWriteBlocked()) return;
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
    const param: GuardParam = {
      id,
      label,
      description: desc,
      file: "",
      apply_mode: mode,
      path,
      value_type: effectiveType,
      default: defaultVal,
      custom: true,
    };
    await contractAddCustomParam(param, fileId);
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
  if (guardWriteBlocked()) return;
  if (!(await ask(t("Delete custom parameter {{id}}?\n\nGuarding stops after deletion. Values already written to ~/.codex/ will not be rolled back; restore manually from ~/.codex/dashi-backups/ if needed.", { id }), { title: t("Delete Custom Parameter"), kind: "warning" }))) {
    return;
  }
  try {
    await contractRemoveCustomParam(id);
    toast(t("Deleted"), "success");
    await refreshGuardView(true);
  } catch (e) {
    toast(t("Delete failed: {{error}}", { error: String(e) }), "error");
  }
}

export async function guardOpenSchemaFile(): Promise<void> {
  try {
    const path = await contractGetSchemaFilePath();
    // shell 插件的 open 可以打开文件所在目录/文件
    await openUrl(path);
  } catch (e) {
    // 回退：复制路径到剪贴板
    try {
      const path = await contractGetSchemaFilePath();
      await navigator.clipboard.writeText(path);
      toast(t("Path copied to clipboard: {{path}}", { path }), "info");
    } catch {
      toast(t("Open failed: {{error}}", { error: String(e) }), "error");
    }
  }
}
