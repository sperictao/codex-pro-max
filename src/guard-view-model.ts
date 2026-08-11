import type {
  BatchAction,
  BatchPreview,
  BatchReport,
  BatchRequest,
  BatchScope,
  DiagnosticCode,
  DiagnosticParams,
  DiagnosticSeverity,
  GuardFile,
  GuardRecoveryStatus,
  GuardView,
  ValidationDiagnostic,
} from "./generated/guard-contracts";

/** The order is part of the Guard workbench contract and must stay stable. */
export const actionOrder = ["apply", "lock", "unlock", "disable"] as const satisfies readonly BatchAction[];

export type GuardAction = (typeof actionOrder)[number];
export type StatusTone = "positive" | "warning" | "negative" | "neutral";

/** Durable transaction stages shown in the workbench. */
export const operationPhases = ["preflight", "snapshot", "write", "verify", "completed", "recovery"] as const;
export type GuardOperationPhase = (typeof operationPhases)[number];

/** Payload exported by Rust; no path or value fields are accepted here. */
export type GuardOperationProgress = {
  batchId: string;
  phase: GuardOperationPhase;
  completed: number;
  total: number;
};

export type GuardActionState = {
  enabled: boolean;
  reason: string | null;
};

export type GuardBatchState = {
  request: BatchRequest;
  operationId: string;
  /**
   * Batch id minted by the backend transaction. It cannot be predicted by the frontend,
   * so the first progress event of an in-flight operation adopts it and later events are
   * matched against it.
   */
  backendBatchId: string | null;
  preview: BatchPreview | null;
  report: BatchReport | null;
  busy: boolean;
  error: string | null;
  phase: GuardOperationPhase;
  progress: number;
};

export type GuardUiState = {
  /** Last successful DTO. Polling failures never erase it. */
  view: GuardView | null;
  viewError: string | null;
  files: GuardFile[] | null;
  filesError: string | null;
  batch: GuardBatchState | null;
  recovery: GuardRecoveryStatus | null;
  recoveryError: string | null;
  expandedGroups: readonly string[];
  expandedRoles: readonly string[];
  /** Role/audit contracts are added independently of the Guard group contract. */
  audit: unknown | null;
  auditError: string | null;
};

export type GuardUiEvent =
  | { type: "view/success"; view: GuardView }
  | { type: "view/error"; error: string }
  | { type: "files/success"; files: GuardFile[] }
  | { type: "files/error"; error: string }
  | { type: "batch/started"; operationId: string; request: BatchRequest }
  | { type: "batch/preview"; operationId: string; preview: BatchPreview; batchId?: string }
  | { type: "batch/progress"; operationId: string; phase: GuardOperationPhase; progress?: number; batchId?: string }
  | { type: "batch/report"; operationId: string; report: BatchReport; batchId?: string }
  | { type: "batch/error"; operationId: string; error: string; batchId?: string }
  | { type: "recovery/success"; recovery: GuardRecoveryStatus }
  | { type: "recovery/error"; error: string }
  | { type: "group/toggle"; groupId: string }
  | { type: "role/toggle"; roleId: string }
  | { type: "audit/success"; result: unknown }
  | { type: "audit/error"; error: string };

const diagnosticCodes = new Set<DiagnosticCode>([
  "invalid_utf_8",
  "nul_byte",
  "toml_invalid",
  "json_invalid",
  "json_duplicate_key",
  "markdown_malformed_marker",
  "markdown_duplicate_marker",
  "markdown_crossing_marker",
  "markdown_unmatched_marker",
  "markdown_unclosed_marker",
  "plan_empty_members",
  "plan_unknown_mode",
  "plan_mode_incompatible",
  "plan_invalid_path",
  "plan_expected_type_mismatch",
  "plan_conflict",
]);

const diagnosticSeverities = new Set<DiagnosticSeverity>(["error"]);
const formats = new Set(["toml", "json", "markdown", "plain_text"]);

export function normalizeOperationPhase(value: unknown): GuardOperationPhase {
  return typeof value === "string" && operationPhases.includes(value as GuardOperationPhase)
    ? value as GuardOperationPhase
    : "preflight";
}

export function normalizeProgress(value: unknown, fallback = 0): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(1, Math.max(0, value > 1 ? value / 100 : value));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function own(value: object, key: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function nullableString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function nullableNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/**
 * Copy only the diagnostic parameters that the Rust contract explicitly exposes.
 * This also prevents inherited properties from becoming UI-visible diagnostics.
 */
export function sanitizeDiagnosticParams(value: unknown): DiagnosticParams {
  if (!isRecord(value) || !own(value, "expectedFormat")) return { expectedFormat: null };
  const expectedFormat = value.expectedFormat;
  return typeof expectedFormat === "string" && formats.has(expectedFormat)
    ? { expectedFormat: expectedFormat as DiagnosticParams["expectedFormat"] }
    : { expectedFormat: null };
}

/**
 * Normalize a backend diagnostic at the UI boundary. Unknown codes are represented
 * by the stable generic plan-conflict code; callers can use `diagnosticMessage` for
 * a generic warning without echoing an untrusted value.
 */
export function sanitizeDiagnostic(value: unknown): ValidationDiagnostic {
  const input = isRecord(value) ? value : {};
  const field = (key: string): unknown => own(input, key) ? input[key] : undefined;
  const codeValue = field("code");
  const severityValue = field("severity");
  const code = typeof codeValue === "string" && diagnosticCodes.has(codeValue as DiagnosticCode)
    ? codeValue as DiagnosticCode
    : "plan_conflict";
  const severity = typeof severityValue === "string" && diagnosticSeverities.has(severityValue as DiagnosticSeverity)
    ? severityValue as DiagnosticSeverity
    : "error";
  return {
    code,
    severity,
    scopeId: nullableString(field("scopeId")) ?? "",
    relativeFile: nullableString(field("relativeFile")),
    field: nullableString(field("field")),
    line: nullableNumber(field("line")),
    column: nullableNumber(field("column")),
    params: sanitizeDiagnosticParams(field("params")),
  };
}

export function sanitizeDiagnostics(value: unknown): ValidationDiagnostic[] {
  return Array.isArray(value) ? value.map(sanitizeDiagnostic) : [];
}

/** Unknown status is never treated as a healthy/green state. */
export function statusTone(status: unknown): StatusTone {
  switch (status) {
    case "match":
    case "healthy":
    case "committed":
    case "applied":
    case "locked":
    case "full":
    case "supported":
    case "resolved":
    case "auto":
    case "success":
      return "positive";
    case "drift":
    case "missing":
    case "starting":
    case "warning":
    case "rejected":
    case "rolled_back":
    case "rollout_only":
    case "pending":
    case "incomplete":
    case "mixed":
      return "warning";
    case "error":
    case "invalid":
    case "unsupported":
    case "critical_recovery":
    case "mismatch":
    case "ambiguous":
    case "failed":
    case "critical":
      return "negative";
    case "disabled":
      return "neutral";
    default:
      return "warning";
  }
}

export const toneForStatus = statusTone;

/** Only global Apply and Lock require a preview/confirmation. */
export function shouldConfirmBatch(scope: BatchScope, action: BatchAction): boolean {
  return scope === "all" && (action === "apply" || action === "lock");
}

export function createGuardUiState(): GuardUiState {
  return {
    view: null,
    viewError: null,
    files: null,
    filesError: null,
    batch: null,
    recovery: null,
    recoveryError: null,
    expandedGroups: [],
    expandedRoles: [],
    audit: null,
    auditError: null,
  };
}

function toggleId(ids: readonly string[], id: string): readonly string[] {
  return ids.includes(id) ? ids.filter((item) => item !== id) : [...ids, id];
}

function batchMatches(state: GuardUiState, operationId: string): boolean {
  return state.batch?.operationId === operationId;
}

function eventBatchMatches(state: GuardUiState, operationId: string, batchId?: string): boolean {
  if (!batchMatches(state, operationId)) return false;
  if (!batchId) return true;
  return batchId === state.batch?.operationId || batchId === state.batch?.preview?.previewId;
}

function normalizeBatchPreview(preview: BatchPreview): BatchPreview {
  return {
    ...preview,
    memberIds: Array.isArray(preview.memberIds) ? [...preview.memberIds] : [],
    diagnostics: sanitizeDiagnostics(preview.diagnostics),
  };
}

function normalizeBatchReport(report: BatchReport): BatchReport {
  return {
    ...report,
    diagnostics: sanitizeDiagnostics(report.diagnostics),
  };
}

/** Pure UI reducer. Refresh events intentionally do not clear unrelated state. */
export function reduceGuardUiState(state: GuardUiState, event: GuardUiEvent): GuardUiState {
  switch (event.type) {
    case "view/success":
      return { ...state, view: event.view, viewError: null };
    case "view/error":
      return { ...state, viewError: event.error };
    case "files/success":
      return { ...state, files: [...event.files], filesError: null };
    case "files/error":
      return { ...state, filesError: event.error };
    case "batch/started":
      return {
        ...state,
        batch: {
          request: event.request,
          operationId: event.operationId,
          backendBatchId: null,
          preview: null,
          report: null,
          busy: true,
          error: null,
          phase: "preflight",
          progress: 0,
        },
      };
    case "batch/preview":
      if (!eventBatchMatches(state, event.operationId, event.batchId)) return state;
      return {
        ...state,
        batch: {
          ...state.batch!,
          preview: normalizeBatchPreview(event.preview),
          error: null,
          phase: "snapshot",
          progress: 0.34,
        },
      };
    case "batch/progress":
      if (!batchMatches(state, event.operationId)) return state;
      // A progress event carrying a different backend batch id belongs to an older
      // operation and must not move this one.
      if (
        event.batchId &&
        state.batch!.backendBatchId &&
        state.batch!.backendBatchId !== event.batchId
      ) {
        return state;
      }
      return {
        ...state,
        batch: {
          ...state.batch!,
          backendBatchId: event.batchId ?? state.batch!.backendBatchId,
          phase: event.phase,
          progress: normalizeProgress(event.progress, state.batch!.progress),
        },
      };
    case "batch/report":
      if (!eventBatchMatches(state, event.operationId, event.batchId)) return state;
      return {
        ...state,
        batch: {
          ...state.batch!,
          report: normalizeBatchReport(event.report),
          busy: false,
          error: null,
          phase: event.report.outcome === "critical_recovery" ? "recovery" : "completed",
          progress: 1,
        },
      };
    case "batch/error":
      if (!eventBatchMatches(state, event.operationId, event.batchId)) return state;
      return {
        ...state,
        batch: {
          ...state.batch!,
          busy: false,
          error: event.error,
          phase: event.error.includes("recovery") ? "recovery" : state.batch!.phase,
          progress: event.error.includes("recovery") ? 1 : state.batch!.progress,
        },
      };
    case "recovery/success":
      return { ...state, recovery: event.recovery, recoveryError: null };
    case "recovery/error":
      return { ...state, recoveryError: event.error };
    case "group/toggle":
      return { ...state, expandedGroups: toggleId(state.expandedGroups, event.groupId) };
    case "role/toggle":
      return { ...state, expandedRoles: toggleId(state.expandedRoles, event.roleId) };
    case "audit/success":
      return { ...state, audit: event.result, auditError: null };
    case "audit/error":
      return { ...state, auditError: event.error };
    default:
      return state;
  }
}

/** Stable, localizable diagnostic categories for the UI; raw backend text is excluded. */
export function diagnosticMessage(diagnostic: ValidationDiagnostic): {
  code: DiagnosticCode;
  file: string | null;
  field: string | null;
  line: number | null;
} {
  return {
    code: diagnostic.code,
    file: diagnostic.relativeFile,
    field: diagnostic.field,
    line: diagnostic.line,
  };
}
