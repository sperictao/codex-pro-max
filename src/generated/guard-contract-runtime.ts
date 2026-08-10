import {
  commands,
  GUARD_CONTRACT,
  type GuardFile,
  type GuardParam,
  type GuardView,
  type JsonValue,
  type Result,
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

export function decodeGuardView(value: unknown): GuardView {
  assertRecord(value, "GuardView");
  assertSchemaVersion(value.schemaVersion);
  assertBoolean(value.enabled, "GuardView.enabled");
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

export async function guardAddFile(name: string, file: string, format: string): Promise<GuardFile> {
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
