import {
  commands,
  GUARD_CONTRACT,
  type GuardFile,
  type GuardFileFormat,
  type GuardGroup,
  type GuardParam,
  type BatchPreview,
  type BatchReport,
  type BatchRequest,
  type GuardRecoveryStatus,
  type ValidationDiagnostic,
  type GuardView,
  type JsonValue,
  type Result,
  type AgentAudit,
  type AuditVerdict,
  type CapabilityModelDto,
  type CapabilitySnapshotDto,
  type OperationAuditRecord,
  type ParentDispatchAudit,
  type RoleDeleteReport,
  type RoleDetailDto,
  type RoleDiscoveryDiagnosticDto,
  type RoleDiscoveryDto,
  type RoleDiscoveryItemDto,
  type RoleListDto,
  type RoleMigrationDto,
  type RoleMigrationResolveInput,
  type RoleReorderInput,
  type LifecycleMigrationChoice,
  type RoleCopyInput,
  type RoleSaveInput,
  type RoleSummaryDto,
  type SubagentAuditResult,
  type TurnAudit,
} from "./guard-contracts";

export class GuardContractError extends Error {
  readonly code = "guard_contract_error";

  constructor(message: string) {
    super(message);
    this.name = "GuardContractError";
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function assertRecord(value: unknown, path: string): asserts value is Record<string, unknown> {
  if (!isRecord(value)) throw new GuardContractError(`${path} must be an object`);
}

function assertString(value: unknown, path: string): asserts value is string {
  if (typeof value !== "string") throw new GuardContractError(`${path} must be a string`);
}

function assertBoolean(value: unknown, path: string): asserts value is boolean {
  if (typeof value !== "boolean") throw new GuardContractError(`${path} must be a boolean`);
}

function assertNullableString(value: unknown, path: string): void {
  if (value !== null && typeof value !== "string") {
    throw new GuardContractError(`${path} must be a string or null`);
  }
}

function assertNullableNumber(value: unknown, path: string): void {
  if (value !== null && (typeof value !== "number" || !Number.isFinite(value))) {
    throw new GuardContractError(`${path} must be a finite number or null`);
  }
}

function assertFiniteNumber(value: unknown, path: string): asserts value is number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new GuardContractError(`${path} must be a finite number`);
  }
}

function assertAllowed(value: unknown, allowed: readonly string[], path: string): asserts value is string {
  assertString(value, path);
  if (!allowed.includes(value)) {
    throw new GuardContractError(`${path} has unsupported value: ${value}`);
  }
}

function assertSchemaVersion(value: unknown): asserts value is number {
  if (value !== GUARD_CONTRACT.schemaVersion) {
    throw new GuardContractError(
      `schemaVersion ${String(value)} is not supported (expected ${GUARD_CONTRACT.schemaVersion})`,
    );
  }
}

const DIAGNOSTIC_CODES = [
  "invalid_utf_8", "nul_byte", "toml_invalid", "json_invalid", "json_duplicate_key",
  "markdown_malformed_marker", "markdown_duplicate_marker", "markdown_crossing_marker",
  "markdown_unmatched_marker", "markdown_unclosed_marker", "plan_empty_members",
  "plan_unknown_mode", "plan_mode_incompatible", "plan_invalid_path",
  "plan_expected_type_mismatch", "plan_conflict",
] as const;

const ROLE_LIFECYCLES = ["disabled", "applied", "locked"] as const;
const ROLE_HEALTHS = ["healthy", "drifted", "invalid", "unsupported", "error"] as const;
const ROLE_DISCOVERY_SYNTAX = ["valid", "invalid"] as const;
const ROLE_DISCOVERY_CAPABILITIES = [
  "supported", "unsupported_model", "unsupported_reasoning_effort",
  "unsupported_multi_agent_version", "unavailable", "invalid",
] as const;
const ROLE_MIGRATION_STATUS = ["auto", "pending", "resolved"] as const;
const ROLE_MIGRATION_CHOICES = ["adopt_live", "adopt_stored", "use_safe_default"] as const;
const ROLE_MIGRATION_REASONS = ["missing_stored", "missing_live", "invalid_stored", "invalid_live", "conflict"] as const;
const ROLE_DIAGNOSTIC_CODES = [
  "invalid_role_id", "invalid_role_file_name", "duplicate_role_id", "default_role_protected",
  "too_many_managed_roles", "invalid_text", "text_too_long", "missing_role_field",
  "invalid_role_field_type", "unknown_role_field", "invalid_role_toml", "invalid_role_order",
  "invalid_role_lifecycle", "unsupported_model", "unsupported_reasoning_effort",
  "unsupported_multi_agent_version", "capability_unavailable", "migration_source_unavailable",
  "migration_source_invalid", "safe_default_invalid", "invalid_migration_choice",
] as const;
const AUDIT_SOURCE_SUPPORT = ["unsupported", "rollout_only", "full"] as const;
const AUDIT_VERDICTS = ["match", "missing", "incomplete", "unsupported", "ambiguous", "mismatch"] as const;
const OPERATION_AUDIT_PHASES = ["preflight", "snapshot", "write", "verify", "completed", "rolled_back", "recovery", "audit"] as const;
const OPERATION_AUDIT_RESULTS = ["success", "failed", "rejected", "busy", "rolled_back", "critical"] as const;

function decodeBatchScope(value: unknown, path: string): void {
  if (value === "all") return;
  assertRecord(value, path);
  const keys = Object.keys(value);
  if (keys.length !== 1 || !["group", "parameter", "role"].includes(keys[0])) {
    throw new GuardContractError(`${path} has unsupported scope`);
  }
  const selected = value[keys[0]];
  assertRecord(selected, `${path}.${keys[0]}`);
  const field = keys[0] === "group" ? "groupId" : keys[0] === "parameter" ? "parameterId" : "roleId";
  assertString(selected[field], `${path}.${keys[0]}.${field}`);
}

function decodeDiagnostic(value: unknown, index: number): ValidationDiagnostic {
  const path = `diagnostics[${index}]`;
  assertRecord(value, path);
  assertAllowed(value.code, DIAGNOSTIC_CODES, `${path}.code`);
  assertAllowed(value.severity, ["error"], `${path}.severity`);
  assertString(value.scopeId, `${path}.scopeId`);
  assertNullableString(value.relativeFile, `${path}.relativeFile`);
  assertNullableString(value.field, `${path}.field`);
  assertNullableNumber(value.line, `${path}.line`);
  assertNullableNumber(value.column, `${path}.column`);
  assertRecord(value.params, `${path}.params`);
  if (value.params.expectedFormat !== null) {
    assertAllowed(value.params.expectedFormat, GUARD_CONTRACT.fileFormats, `${path}.params.expectedFormat`);
  }
  return value as unknown as ValidationDiagnostic;
}

function decodeDiagnostics(value: unknown): ValidationDiagnostic[] {
  if (!Array.isArray(value)) throw new GuardContractError("diagnostics must be an array");
  return value.map(decodeDiagnostic);
}

function decodeRoleSummary(value: unknown, path: string): RoleSummaryDto {
  assertRecord(value, path);
  assertString(value.id, `${path}.id`);
  assertString(value.displayName, `${path}.displayName`);
  assertString(value.purpose, `${path}.purpose`);
  assertString(value.selectionCriteria, `${path}.selectionCriteria`);
  assertString(value.model, `${path}.model`);
  assertString(value.effort, `${path}.effort`);
  assertFiniteNumber(value.order, `${path}.order`);
  assertAllowed(value.lifecycle, ROLE_LIFECYCLES, `${path}.lifecycle`);
  assertAllowed(value.health, ROLE_HEALTHS, `${path}.health`);
  assertBoolean(value.actualFilePresent, `${path}.actualFilePresent`);
  return value as unknown as RoleSummaryDto;
}

export function decodeRoleDetail(value: unknown): RoleDetailDto {
  assertRecord(value, "RoleDetailDto");
  assertString(value.id, "RoleDetailDto.id");
  assertString(value.displayName, "RoleDetailDto.displayName");
  assertString(value.purpose, "RoleDetailDto.purpose");
  assertString(value.selectionCriteria, "RoleDetailDto.selectionCriteria");
  assertString(value.model, "RoleDetailDto.model");
  assertString(value.effort, "RoleDetailDto.effort");
  assertString(value.instructions, "RoleDetailDto.instructions");
  assertAllowed(value.lifecycle, ROLE_LIFECYCLES, "RoleDetailDto.lifecycle");
  assertAllowed(value.health, ROLE_HEALTHS, "RoleDetailDto.health");
  assertBoolean(value.actualFilePresent, "RoleDetailDto.actualFilePresent");
  assertFiniteNumber(value.policyRevision, "RoleDetailDto.policyRevision");
  assertString(value.policyHash, "RoleDetailDto.policyHash");
  return value as unknown as RoleDetailDto;
}

function decodeRoleDiscoveryDiagnostic(value: unknown, path: string): RoleDiscoveryDiagnosticDto {
  assertRecord(value, path);
  assertAllowed(value.code, ROLE_DIAGNOSTIC_CODES, `${path}.code`);
  assertNullableString(value.field, `${path}.field`);
  return value as unknown as RoleDiscoveryDiagnosticDto;
}

function decodeRoleDiscoveryItem(value: unknown, path: string): RoleDiscoveryItemDto {
  assertRecord(value, path);
  assertString(value.id, `${path}.id`);
  assertString(value.relativeFileName, `${path}.relativeFileName`);
  assertAllowed(value.syntax, ROLE_DISCOVERY_SYNTAX, `${path}.syntax`);
  assertAllowed(value.capability, ROLE_DISCOVERY_CAPABILITIES, `${path}.capability`);
  assertBoolean(value.canAdopt, `${path}.canAdopt`);
  assertBoolean(value.managed, `${path}.managed`);
  if (!Array.isArray(value.diagnostics)) throw new GuardContractError(`${path}.diagnostics must be an array`);
  value.diagnostics.forEach((diagnostic, index) => decodeRoleDiscoveryDiagnostic(diagnostic, `${path}.diagnostics[${index}]`));
  return value as unknown as RoleDiscoveryItemDto;
}

export function decodeRoleList(value: unknown): RoleListDto {
  assertRecord(value, "RoleListDto");
  if (!Array.isArray(value.roles)) throw new GuardContractError("RoleListDto.roles must be an array");
  value.roles.forEach((role, index) => decodeRoleSummary(role, `RoleListDto.roles[${index}]`));
  return value as unknown as RoleListDto;
}

export function decodeRoleDiscovery(value: unknown): RoleDiscoveryDto {
  assertRecord(value, "RoleDiscoveryDto");
  if (!Array.isArray(value.roles)) throw new GuardContractError("RoleDiscoveryDto.roles must be an array");
  value.roles.forEach((role, index) => decodeRoleDiscoveryItem(role, `RoleDiscoveryDto.roles[${index}]`));
  return value as unknown as RoleDiscoveryDto;
}

export function decodeRoleDeleteReport(value: unknown): RoleDeleteReport {
  assertRecord(value, "RoleDeleteReport");
  assertString(value.id, "RoleDeleteReport.id");
  assertBoolean(value.fileRemoved, "RoleDeleteReport.fileRemoved");
  assertBoolean(value.directoryUpdated, "RoleDeleteReport.directoryUpdated");
  assertFiniteNumber(value.filesRemoved, "RoleDeleteReport.filesRemoved");
  assertNullableString(value.backupSummary, "RoleDeleteReport.backupSummary");
  return value as unknown as RoleDeleteReport;
}

function decodeCapabilityModel(value: unknown, path: string): CapabilityModelDto {
  assertRecord(value, path);
  assertString(value.slug, `${path}.slug`);
  assertString(value.multiAgentVersion, `${path}.multiAgentVersion`);
  if (!Array.isArray(value.efforts)) throw new GuardContractError(`${path}.efforts must be an array`);
  value.efforts.forEach((effort, index) => assertString(effort, `${path}.efforts[${index}]`));
  return value as unknown as CapabilityModelDto;
}

export function decodeCapabilitySnapshot(value: unknown): CapabilitySnapshotDto {
  assertRecord(value, "CapabilitySnapshotDto");
  assertBoolean(value.available, "CapabilitySnapshotDto.available");
  assertNullableString(value.executableIdentity, "CapabilitySnapshotDto.executableIdentity");
  assertNullableNumber(value.fetchedAtMs, "CapabilitySnapshotDto.fetchedAtMs");
  assertNullableString(value.error, "CapabilitySnapshotDto.error");
  if (!Array.isArray(value.models)) throw new GuardContractError("CapabilitySnapshotDto.models must be an array");
  value.models.forEach((model, index) => decodeCapabilityModel(model, `CapabilitySnapshotDto.models[${index}]`));
  return value as unknown as CapabilitySnapshotDto;
}

export function decodeRoleMigration(value: unknown): RoleMigrationDto {
  assertRecord(value, "RoleMigrationDto");
  assertAllowed(value.status, ROLE_MIGRATION_STATUS, "RoleMigrationDto.status");
  assertNullableString(value.reason, "RoleMigrationDto.reason");
  if (value.reason !== null) assertAllowed(value.reason, ROLE_MIGRATION_REASONS, "RoleMigrationDto.reason");
  if (!Array.isArray(value.choices)) throw new GuardContractError("RoleMigrationDto.choices must be an array");
  value.choices.forEach((choice, index) => assertAllowed(choice, ROLE_MIGRATION_CHOICES, `RoleMigrationDto.choices[${index}]`));
  if (value.role !== null) decodeRoleSummary(value.role, "RoleMigrationDto.role");
  return value as unknown as RoleMigrationDto;
}

function decodeAuditVerdict(value: unknown, path: string): asserts value is AuditVerdict {
  assertAllowed(value, AUDIT_VERDICTS, path);
}

function decodeParentDispatchAudit(value: unknown, path: string): ParentDispatchAudit {
  assertRecord(value, path);
  decodeAuditVerdict(value.verdict, `${path}.verdict`);
  assertNullableString(value.observedAgentType, `${path}.observedAgentType`);
  assertNullableString(value.observedForkTurns, `${path}.observedForkTurns`);
  assertNullableString(value.overrideModel, `${path}.overrideModel`);
  assertNullableString(value.overrideEffort, `${path}.overrideEffort`);
  return value as unknown as ParentDispatchAudit;
}

function decodeTurnAudit(value: unknown, path: string): TurnAudit {
  assertRecord(value, path);
  assertString(value.turnId, `${path}.turnId`);
  assertNullableNumber(value.startedAtMs, `${path}.startedAtMs`);
  assertNullableString(value.model, `${path}.model`);
  assertNullableString(value.effort, `${path}.effort`);
  assertNullableString(value.multiAgentVersion, `${path}.multiAgentVersion`);
  decodeAuditVerdict(value.clientVerdict, `${path}.clientVerdict`);
  assertNullableString(value.outboundRequestedModel, `${path}.outboundRequestedModel`);
  assertFiniteNumber(value.outboundEvidenceCount, `${path}.outboundEvidenceCount`);
  decodeAuditVerdict(value.outboundVerdict, `${path}.outboundVerdict`);
  decodeAuditVerdict(value.verdict, `${path}.verdict`);
  return value as unknown as TurnAudit;
}

function decodeAgentAudit(value: unknown, path: string): AgentAudit {
  assertRecord(value, path);
  assertString(value.threadId, `${path}.threadId`);
  assertString(value.roleId, `${path}.roleId`);
  assertNullableNumber(value.expectedPolicyRevision, `${path}.expectedPolicyRevision`);
  assertNullableString(value.expectedPolicyHash, `${path}.expectedPolicyHash`);
  assertNullableString(value.expectedModel, `${path}.expectedModel`);
  assertNullableString(value.expectedEffort, `${path}.expectedEffort`);
  decodeParentDispatchAudit(value.parent, `${path}.parent`);
  if (!Array.isArray(value.turns)) throw new GuardContractError(`${path}.turns must be an array`);
  value.turns.forEach((turn, index) => decodeTurnAudit(turn, `${path}.turns[${index}]`));
  decodeAuditVerdict(value.verdict, `${path}.verdict`);
  return value as unknown as AgentAudit;
}

export function decodeSubagentAudit(value: unknown): SubagentAuditResult {
  assertRecord(value, "SubagentAuditResult");
  assertSchemaVersion(value.schemaVersion);
  assertFiniteNumber(value.checkedAtMs, "SubagentAuditResult.checkedAtMs");
  assertAllowed(value.sourceSupport, AUDIT_SOURCE_SUPPORT, "SubagentAuditResult.sourceSupport");
  if (!Array.isArray(value.agents)) throw new GuardContractError("SubagentAuditResult.agents must be an array");
  value.agents.forEach((agent, index) => decodeAgentAudit(agent, `SubagentAuditResult.agents[${index}]`));
  if (!Array.isArray(value.notices)) throw new GuardContractError("SubagentAuditResult.notices must be an array");
  value.notices.forEach((notice, index) => {
    const path = `SubagentAuditResult.notices[${index}]`;
    assertRecord(notice, path);
    assertString(notice.code, `${path}.code`);
  });
  return value as unknown as SubagentAuditResult;
}

export function decodeOperationAuditRecord(value: unknown, path = "OperationAuditRecord"): OperationAuditRecord {
  assertRecord(value, path);
  assertSchemaVersion(value.schemaVersion);
  assertFiniteNumber(value.atMs, `${path}.atMs`);
  assertString(value.batchId, `${path}.batchId`);
  assertString(value.scope, `${path}.scope`);
  assertNullableString(value.relativeFile, `${path}.relativeFile`);
  assertAllowed(value.phase, OPERATION_AUDIT_PHASES, `${path}.phase`);
  assertAllowed(value.result, OPERATION_AUDIT_RESULTS, `${path}.result`);
  assertNullableString(value.errorCode, `${path}.errorCode`);
  for (const field of ["changed", "unchanged", "files"] as const) {
    assertFiniteNumber(value[field], `${path}.${field}`);
  }
  assertNullableString(value.roleId, `${path}.roleId`);
  assertNullableString(value.model, `${path}.model`);
  assertNullableString(value.effort, `${path}.effort`);
  return value as unknown as OperationAuditRecord;
}

export const decodeRoleDetailDto = decodeRoleDetail;
export const decodeRoleListDto = decodeRoleList;
export const decodeRoleDiscoveryDto = decodeRoleDiscovery;
export const decodeRoleDeleteReportDto = decodeRoleDeleteReport;
export const decodeCapabilitySnapshotDto = decodeCapabilitySnapshot;
export const decodeRoleMigrationDto = decodeRoleMigration;
export const decodeSubagentAuditResult = decodeSubagentAudit;
export const decodeOperationAuditRecords = (value: unknown): OperationAuditRecord[] => {
  if (!Array.isArray(value)) throw new GuardContractError("OperationAuditRecord[] must be an array");
  return value.map((record, index) => decodeOperationAuditRecord(record, `OperationAuditRecord[${index}]`));
};

function unwrap<T>(result: Result<T, string>): T {
  if (result.status === "error") throw new GuardContractError(result.error);
  return result.data;
}

function decodeGuardParamView(value: unknown, index: number): void {
  const path = `groups[].params[${index}]`;
  assertRecord(value, path);
  assertString(value.id, `${path}.id`);
  assertString(value.label, `${path}.label`);
  assertString(value.description, `${path}.description`);
  assertAllowed(value.applyMode, GUARD_CONTRACT.applyModes, `${path}.applyMode`);
  assertAllowed(value.valueType, GUARD_CONTRACT.valueTypes, `${path}.valueType`);
  assertString(value.path, `${path}.path`);
  assertBoolean(value.applied, `${path}.applied`);
  assertBoolean(value.locked, `${path}.locked`);
  assertAllowed(value.status, GUARD_CONTRACT.paramStatuses, `${path}.status`);
}

function decodeGuardGroup(value: unknown, path: string): GuardGroup {
  assertRecord(value, path);
  assertString(value.id, `${path}.id`);
  assertString(value.name, `${path}.name`);
  if (value.builtin !== undefined) assertBoolean(value.builtin, `${path}.builtin`);
  if (value.order !== undefined) assertFiniteNumber(value.order, `${path}.order`);
  return value as unknown as GuardGroup;
}

function decodeGuardGroups(value: unknown, path: string): GuardGroup[] {
  if (!Array.isArray(value)) throw new GuardContractError(`${path} must be an array`);
  return value.map((group, index) => decodeGuardGroup(group, `${path}[${index}]`));
}

function decodeGuardParam(value: unknown, path: string): GuardParam {
  assertRecord(value, path);
  assertString(value.id, `${path}.id`);
  assertString(value.label, `${path}.label`);
  assertString(value.file, `${path}.file`);
  assertString(value.apply_mode, `${path}.apply_mode`);
  assertString(value.value_type, `${path}.value_type`);
  if (value.path !== undefined) assertString(value.path, `${path}.path`);
  if (value.group_id !== undefined && value.group_id !== null) assertString(value.group_id, `${path}.group_id`);
  if (value.custom !== undefined) assertBoolean(value.custom, `${path}.custom`);
  return value as unknown as GuardParam;
}

export function decodeGuardView(value: unknown): GuardView {
  assertRecord(value, "GuardView");
  assertSchemaVersion(value.schemaVersion);
  assertBoolean(value.enabled, "GuardView.enabled");
  if (!Array.isArray(value.pendingFormatMigrations)) throw new GuardContractError("GuardView.pendingFormatMigrations must be an array");
  value.pendingFormatMigrations.forEach((pending, index) => {
    const path = `pendingFormatMigrations[${index}]`;
    assertRecord(pending, path);
    assertString(pending.id, `${path}.id`);
    assertString(pending.name, `${path}.name`);
    assertString(pending.file, `${path}.file`);
    assertBoolean(pending.builtin, `${path}.builtin`);
    if (pending.detection !== null && pending.detection !== undefined) {
      assertRecord(pending.detection, `${path}.detection`);
      if (pending.detection.path !== null && pending.detection.path !== undefined) assertString(pending.detection.path, `${path}.detection.path`);
      if (typeof pending.detection.at !== "number" || !Number.isFinite(pending.detection.at)) throw new GuardContractError(`${path}.detection.at must be a finite number`);
    }
    if (pending.rawFormat !== null && pending.rawFormat !== undefined) assertString(pending.rawFormat, `${path}.rawFormat`);
    if (!Array.isArray(pending.candidates)) throw new GuardContractError(`${path}.candidates must be an array`);
    pending.candidates.forEach((candidate, candidateIndex) => assertAllowed(candidate, GUARD_CONTRACT.fileFormats, `${path}.candidates[${candidateIndex}]`));
  });
  if (!Array.isArray(value.groups)) throw new GuardContractError("GuardView.groups must be an array");
  value.groups.forEach((group, groupIndex) => {
    const path = `groups[${groupIndex}]`;
    assertRecord(group, path);
    assertString(group.id, `${path}.id`);
    assertString(group.name, `${path}.name`);
    assertString(group.file, `${path}.file`);
    assertAllowed(group.format, GUARD_CONTRACT.fileFormats, `${path}.format`);
    assertBoolean(group.builtin, `${path}.builtin`);
    if (!Array.isArray(group.params)) throw new GuardContractError(`${path}.params must be an array`);
    group.params.forEach((param, paramIndex) => decodeGuardParamView(param, paramIndex));
    if (group.files !== undefined) {
      if (!Array.isArray(group.files)) throw new GuardContractError(`${path}.files must be an array`);
      group.files.forEach((file, fileIndex) => {
        const filePath = `${path}.files[${fileIndex}]`;
        assertRecord(file, filePath);
        assertString(file.file, `${filePath}.file`);
        assertAllowed(file.format, GUARD_CONTRACT.fileFormats, `${filePath}.format`);
      });
    }
  });
  return value as unknown as GuardView;
}

export function decodeGuardFile(value: unknown): GuardFile {
  assertRecord(value, "GuardFile");
  assertString(value.id, "GuardFile.id");
  assertString(value.name, "GuardFile.name");
  assertString(value.file, "GuardFile.file");
  assertAllowed(value.format, GUARD_CONTRACT.fileFormats, "GuardFile.format");
  assertBoolean(value.builtin, "GuardFile.builtin");
  return value as unknown as GuardFile;
}

export function decodeGuardFiles(value: unknown): GuardFile[] {
  if (!Array.isArray(value)) throw new GuardContractError("GuardFile[] must be an array");
  return value.map(decodeGuardFile);
}

export async function guardGetView(): Promise<GuardView> {
  return decodeGuardView(unwrap(await commands.guardGetView()));
}

export async function guardSetEnabled(enabled: boolean): Promise<void> {
  unwrap(await commands.guardSetEnabled(enabled));
}

export async function guardSetValue(id: string, value: JsonValue): Promise<void> {
  unwrap(await commands.guardSetValue(id, value));
}

export async function guardApply(id: string): Promise<void> {
  unwrap(await commands.guardApply(id));
}

export async function guardSetApplied(id: string, applied: boolean): Promise<void> {
  unwrap(await commands.guardSetApplied(id, applied));
}

export async function guardSetLocked(id: string, locked: boolean): Promise<void> {
  unwrap(await commands.guardSetLocked(id, locked));
}

export async function guardAddCustomParam(param: GuardParam, fileId: string): Promise<void> {
  unwrap(await commands.guardAddCustomParam(param, fileId));
}

export async function guardRemoveCustomParam(id: string): Promise<void> {
  unwrap(await commands.guardRemoveCustomParam(id));
}

export async function guardGetSchemaFilePath(): Promise<string> {
  return unwrap(await commands.guardGetSchemaFilePath());
}

export async function guardGetFiles(): Promise<GuardFile[]> {
  return decodeGuardFiles(unwrap(await commands.guardGetFiles()));
}

export async function guardAddFile(name: string, file: string, format: GuardFileFormat): Promise<GuardFile> {
  return decodeGuardFile(unwrap(await commands.guardAddFile(name, file, format)));
}

export async function guardUpdateFile(id: string, name: string, file: string): Promise<GuardFile> {
  return decodeGuardFile(unwrap(await commands.guardUpdateFile(id, name, file)));
}

export async function guardRemoveFile(id: string): Promise<void> {
  unwrap(await commands.guardRemoveFile(id));
}

export async function guardDetectFile(id: string): Promise<GuardFile> {
  return decodeGuardFile(unwrap(await commands.guardDetectFile(id)));
}

export async function guardRelativizePickedPath(absPath: string): Promise<string> {
  return unwrap(await commands.guardRelativizePickedPath(absPath));
}

export function decodeGuardRecoveryStatus(value: unknown): GuardRecoveryStatus {
  assertRecord(value, "GuardRecoveryStatus");
  assertBoolean(value.blocked, "GuardRecoveryStatus.blocked");
  assertNullableString(value.code, "GuardRecoveryStatus.code");
  return value as unknown as GuardRecoveryStatus;
}

export function decodeBatchPreview(value: unknown): BatchPreview {
  assertRecord(value, "BatchPreview");
  assertSchemaVersion(value.schemaVersion);
  assertString(value.previewId, "BatchPreview.previewId");
  assertString(value.revision, "BatchPreview.revision");
  assertAllowed(value.action, ["apply", "lock", "unlock", "disable"], "BatchPreview.action");
  decodeBatchScope(value.scope, "BatchPreview.scope");
  if (!Array.isArray(value.memberIds)) throw new GuardContractError("BatchPreview.memberIds must be an array");
  value.memberIds.forEach((member, index) => assertString(member, `BatchPreview.memberIds[${index}]`));
  for (const field of ["affectedMembers", "affectedFiles", "changed", "unchanged"] as const) {
    if (typeof value[field] !== "number" || !Number.isFinite(value[field])) {
      throw new GuardContractError(`BatchPreview.${field} must be a finite number`);
    }
  }
  if (typeof value.files !== "number" || !Number.isFinite(value.files)) {
    throw new GuardContractError("BatchPreview.files must be a finite number");
  }
  assertBoolean(value.eligible, "BatchPreview.eligible");
  decodeDiagnostics(value.blockers);
  decodeDiagnostics(value.diagnostics);
  return value as unknown as BatchPreview;
}

export function decodeBatchReport(value: unknown): BatchReport {
  assertRecord(value, "BatchReport");
  assertSchemaVersion(value.schemaVersion);
  assertString(value.batchId, "BatchReport.batchId");
  assertAllowed(value.outcome, ["committed", "rejected", "rolled_back", "critical_recovery"], "BatchReport.outcome");
  for (const field of ["changed", "unchanged", "files"] as const) {
    if (typeof value[field] !== "number" || !Number.isFinite(value[field])) {
      throw new GuardContractError(`BatchReport.${field} must be a finite number`);
    }
  }
  decodeDiagnostics(value.diagnostics);
  return value as unknown as BatchReport;
}

export async function guardGetRecoveryStatus(): Promise<GuardRecoveryStatus> {
  return decodeGuardRecoveryStatus(unwrap(await commands.guardGetRecoveryStatus()));
}

export async function guardRetryRecovery(): Promise<void> {
  unwrap(await commands.guardRetryRecovery());
}

export async function guardPreviewBatch(request: BatchRequest): Promise<BatchPreview> {
  return decodeBatchPreview(unwrap(await commands.guardPreviewBatch(request)));
}

export async function guardExecuteBatch(request: BatchRequest, previewId: string | null): Promise<BatchReport> {
  return decodeBatchReport(unwrap(await commands.guardExecuteBatch(request, previewId)));
}

export async function guardGroupCreate(name: string): Promise<GuardGroup> {
  return decodeGuardGroup(unwrap(await commands.guardGroupCreate(name)), "GuardGroup");
}

export async function guardGroupRename(id: string, name: string): Promise<GuardGroup> {
  return decodeGuardGroup(unwrap(await commands.guardGroupRename(id, name)), "GuardGroup");
}

export async function guardGroupReorder(ids: string[]): Promise<GuardGroup[]> {
  return decodeGuardGroups(unwrap(await commands.guardGroupReorder(ids)), "GuardGroup[]");
}

export async function guardGroupDelete(id: string): Promise<void> {
  unwrap(await commands.guardGroupDelete(id));
}

export async function guardParameterMove(parameterId: string, groupId: string): Promise<GuardParam> {
  return decodeGuardParam(unwrap(await commands.guardParameterMove(parameterId, groupId)), "GuardParam");
}

export async function guardLifecycleMigrationResolve(
  parameterId: string,
  choice: LifecycleMigrationChoice,
): Promise<void> {
  unwrap(await commands.guardLifecycleMigrationResolve(parameterId, choice));
}

export async function guardFileFormatMigrationResolve(
  fileId: string,
  format: GuardFileFormat,
): Promise<GuardFile> {
  return decodeGuardFile(unwrap(await commands.guardFileFormatMigrationResolve(fileId, format)));
}

export async function guardRoleGet(id: string): Promise<RoleDetailDto> {
  return decodeRoleDetail(unwrap(await commands.guardRoleGet(id)));
}

export async function guardRoleList(): Promise<RoleListDto> {
  return decodeRoleList(unwrap(await commands.guardRoleList()));
}

export async function guardRoleSave(input: RoleSaveInput): Promise<RoleDetailDto> {
  return decodeRoleDetail(unwrap(await commands.guardRoleSave(input)));
}

export async function guardRoleCopy(source: string, input: RoleCopyInput): Promise<RoleDetailDto> {
  return decodeRoleDetail(unwrap(await commands.guardRoleCopy(source, input)));
}

export async function guardRoleReorder(input: RoleReorderInput): Promise<RoleListDto> {
  return decodeRoleList(unwrap(await commands.guardRoleReorder(input)));
}

export async function guardRoleDiscover(): Promise<RoleDiscoveryDto> {
  return decodeRoleDiscovery(unwrap(await commands.guardRoleDiscover()));
}

export async function guardRoleAdopt(id: string): Promise<RoleDetailDto> {
  return decodeRoleDetail(unwrap(await commands.guardRoleAdopt(id)));
}

export async function guardRoleStopManaging(id: string): Promise<RoleDiscoveryDto> {
  return decodeRoleDiscovery(unwrap(await commands.guardRoleStopManaging(id)));
}

export async function guardRoleDelete(id: string): Promise<RoleDeleteReport> {
  return decodeRoleDeleteReport(unwrap(await commands.guardRoleDelete(id)));
}

export async function guardCapabilityGet(): Promise<CapabilitySnapshotDto> {
  return decodeCapabilitySnapshot(unwrap(await commands.guardCapabilityGet()));
}

export async function guardCapabilityRefresh(): Promise<CapabilitySnapshotDto> {
  return decodeCapabilitySnapshot(unwrap(await commands.guardCapabilityRefresh()));
}

export async function guardRoleMigrationPlan(safeDefaultToml: string): Promise<RoleMigrationDto> {
  return decodeRoleMigration(unwrap(await commands.guardRoleMigrationPlan(safeDefaultToml)));
}

export async function guardRoleMigrationResolve(input: RoleMigrationResolveInput): Promise<RoleMigrationDto> {
  return decodeRoleMigration(unwrap(await commands.guardRoleMigrationResolve(input)));
}

export async function guardRunSubagentAudit(): Promise<SubagentAuditResult> {
  return decodeSubagentAudit(unwrap(await commands.guardRunSubagentAudit()));
}

export async function guardOperationAuditList(): Promise<OperationAuditRecord[]> {
  return decodeOperationAuditRecords(unwrap(await commands.guardOperationAuditList()));
}
