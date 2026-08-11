import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import test from "node:test";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const outputDir = mkdtempSync(join(tmpdir(), "dashi-guard-view-model-test-"));
process.on("exit", () => rmSync(outputDir, { recursive: true, force: true }));

execFileSync(process.execPath, [
  join(root, "node_modules/typescript/bin/tsc"),
  join(root, "src/guard-view-model.ts"),
  "--target", "ES2022",
  "--module", "CommonJS",
  "--moduleResolution", "node",
  "--lib", "ES2022,DOM",
  "--strict",
  "--skipLibCheck",
  "--outDir", outputDir,
], { cwd: root, stdio: "inherit" });

const require = createRequire(import.meta.url);
const viewModel = require(join(outputDir, "guard-view-model.js"));
const fixture = JSON.parse(readFileSync(
  join(root, "src-tauri/src/codex_guard/fixtures/contracts/guard-view.json"),
  "utf8",
));

test("Guard action order and confirmation matrix stay stable", () => {
  assert.deepEqual(viewModel.actionOrder, ["apply", "lock", "unlock", "disable"]);
  assert.equal(viewModel.shouldConfirmBatch("all", "apply"), true);
  assert.equal(viewModel.shouldConfirmBatch("all", "lock"), true);
  assert.equal(viewModel.shouldConfirmBatch("all", "unlock"), false);
  assert.equal(viewModel.shouldConfirmBatch({ groupId: "g" }, "apply"), false);
  assert.equal(viewModel.shouldConfirmBatch({ roleId: "r" }, "lock"), false);
});

test("unknown status and diagnostic input fail closed", () => {
  assert.equal(viewModel.statusTone("future_status"), "warning");
  const inherited = Object.create({ expectedFormat: "json" });
  assert.deepEqual(viewModel.sanitizeDiagnosticParams(inherited), { expectedFormat: null });
  assert.equal(viewModel.sanitizeDiagnostic({ code: "future", severity: "success" }).code, "plan_conflict");
});

test("refresh errors preserve the last view, recovery, audit, and expanded state", () => {
  let state = viewModel.createGuardUiState();
  state = viewModel.reduceGuardUiState(state, { type: "view/success", view: fixture });
  state = viewModel.reduceGuardUiState(state, { type: "recovery/success", recovery: { blocked: true, code: "recovery_failed" } });
  state = viewModel.reduceGuardUiState(state, { type: "audit/success", result: { schemaVersion: 1 } });
  state = viewModel.reduceGuardUiState(state, { type: "group/toggle", groupId: "config" });
  state = viewModel.reduceGuardUiState(state, { type: "view/error", error: "network" });
  assert.equal(state.view, fixture);
  assert.equal(state.recovery.blocked, true);
  assert.deepEqual(state.audit, { schemaVersion: 1 });
  assert.deepEqual(state.expandedGroups, ["config"]);
  assert.equal(state.viewError, "network");
});

test("stale batch events cannot replace a newer operation", () => {
  let state = viewModel.createGuardUiState();
  state = viewModel.reduceGuardUiState(state, {
    type: "batch/started",
    operationId: "new",
    request: { schemaVersion: 1, scope: "all", action: "apply" },
  });
  const stale = viewModel.reduceGuardUiState(state, {
    type: "batch/report",
    operationId: "old",
    report: { schemaVersion: 1, batchId: "old", outcome: "committed", changed: 1, unchanged: 0, files: 1, diagnostics: [] },
  });
  assert.equal(stale.batch.operationId, "new");
  assert.equal(stale.batch.report, null);
  const current = viewModel.reduceGuardUiState(state, {
    type: "batch/report",
    operationId: "new",
    report: { schemaVersion: 1, batchId: "new", outcome: "committed", changed: 1, unchanged: 0, files: 1, diagnostics: [] },
  });
  assert.equal(current.batch.busy, false);
  assert.equal(current.batch.report.outcome, "committed");
});

test("role expansion is independently retained", () => {
  let state = viewModel.createGuardUiState();
  state = viewModel.reduceGuardUiState(state, { type: "role/toggle", roleId: "reviewer" });
  assert.deepEqual(state.expandedRoles, ["reviewer"]);
  state = viewModel.reduceGuardUiState(state, { type: "role/toggle", roleId: "reviewer" });
  assert.deepEqual(state.expandedRoles, []);
});

test("batch progress keeps six ordered phases and ignores stale progress", () => {
  assert.deepEqual(viewModel.operationPhases, ["preflight", "snapshot", "write", "verify", "completed", "recovery"]);
  let state = viewModel.createGuardUiState();
  state = viewModel.reduceGuardUiState(state, {
    type: "batch/started",
    operationId: "current",
    request: { schemaVersion: 1, scope: "all", action: "apply" },
  });
  state = viewModel.reduceGuardUiState(state, {
    type: "batch/progress",
    operationId: "current",
    phase: "write",
    progress: 58,
  });
  assert.equal(state.batch.phase, "write");
  assert.equal(state.batch.progress, 0.58);
  const stale = viewModel.reduceGuardUiState(state, {
    type: "batch/progress",
    operationId: "old",
    phase: "recovery",
    progress: 1,
  });
  assert.equal(stale.batch.phase, "write");
});

test("progress values clamp to a safe range", () => {
  assert.equal(viewModel.normalizeProgress(-4), 0);
  assert.equal(viewModel.normalizeProgress(140), 1);
  assert.equal(viewModel.normalizeOperationPhase("future"), "preflight");
});

test("backend batch ids bind to the in-flight operation and reject foreign batches", () => {
  let state = viewModel.createGuardUiState();
  state = viewModel.reduceGuardUiState(state, {
    type: "batch/started",
    operationId: "op-1",
    request: { schemaVersion: 1, scope: "all", action: "apply" },
  });
  assert.equal(state.batch.backendBatchId, null);

  // The backend mints `{pid}-{nanos}-{serial}`, which can never equal the frontend id.
  // The first progress event must still be accepted and pin that id.
  state = viewModel.reduceGuardUiState(state, {
    type: "batch/progress",
    operationId: "op-1",
    batchId: "4213-1723459200000000-7",
    phase: "write",
    progress: 0.5,
  });
  assert.equal(state.batch.backendBatchId, "4213-1723459200000000-7");
  assert.equal(state.batch.phase, "write");

  // A different backend batch belongs to an older operation and must be ignored.
  const foreign = viewModel.reduceGuardUiState(state, {
    type: "batch/progress",
    operationId: "op-1",
    batchId: "4213-1723459100000000-3",
    phase: "recovery",
    progress: 1,
  });
  assert.equal(foreign.batch.phase, "write");

  // Further events from the pinned batch continue to advance it.
  const next = viewModel.reduceGuardUiState(state, {
    type: "batch/progress",
    operationId: "op-1",
    batchId: "4213-1723459200000000-7",
    phase: "verify",
    progress: 0.9,
  });
  assert.equal(next.batch.phase, "verify");
});
