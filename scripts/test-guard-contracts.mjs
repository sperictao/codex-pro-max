import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createRequire, Module } from "node:module";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import test from "node:test";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const generatedPath = join(root, "src", "generated", "guard-contracts.ts");
const generatedSource = readFileSync(generatedPath, "utf8");
const guardSource = readFileSync(join(root, "src", "guard.ts"), "utf8");
const indexSource = readFileSync(join(root, "index.html"), "utf8");
const fixturePath = join(root, "src-tauri", "src", "codex_guard", "fixtures", "contracts", "guard-view.json");
const fixture = JSON.parse(readFileSync(fixturePath, "utf8"));
const registry = JSON.parse(readFileSync(join(root, "src-tauri", "src", "codex_guard", "fixtures", "contracts", "command-registry.json"), "utf8"));

const runtimeDir = mkdtempSync(join(tmpdir(), "dashi-guard-contract-test-"));
process.on("exit", () => rmSync(runtimeDir, { recursive: true, force: true }));
execFileSync(process.execPath, [
  join(root, "node_modules/typescript/bin/tsc"),
  join(root, "src/generated/guard-contract-runtime.ts"),
  "--target",
  "ES2022",
  "--module",
  "CommonJS",
  "--moduleResolution",
  "node",
  "--strict",
  "--skipLibCheck",
  "--outDir",
  runtimeDir,
], { cwd: root, stdio: "inherit" });
process.env.NODE_PATH = join(root, "node_modules");
Module._initPaths();
const require = createRequire(import.meta.url);
const runtime = require(join(runtimeDir, "guard-contract-runtime.js"));

test("Guard contracts have a checked-in generated source", () => {
  assert.equal(existsSync(generatedPath), true);
});

test("generated command registry covers exactly the Guard command fixture", () => {
  const names = [...generatedSource.matchAll(/^async (guard\w+)\(/gm)].map((match) => match[1]);
  const camel = (name) => name.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase());
  assert.deepEqual(names, registry.commands.map(camel));
  for (const command of registry.commands) {
    assert.match(generatedSource, new RegExp(`TAURI_INVOKE\\("${command}"(?:,|\\))`));
  }
});

test("generated Guard file format contract is canonical", () => {
  assert.match(generatedSource, /fileFormats.*toml.*json.*markdown.*plain_text/);
  assert.doesNotMatch(generatedSource, /fileFormats.*\"md\"/);
  assert.match(indexSource, /option value="markdown"/);
  assert.match(indexSource, /option value="plain_text"/);
  assert.doesNotMatch(indexSource, /option value="md"/);
});

test("Guard UI does not handwrite Tauri invoke calls", () => {
  assert.doesNotMatch(guardSource, /@tauri-apps\/api\/core/);
  assert.doesNotMatch(guardSource, /invoke\(\s*["']guard_/);
  assert.match(guardSource, /apply_mode:/);
  assert.match(guardSource, /value_type:/);
  assert.doesNotMatch(guardSource, /applyMode:/);
  assert.doesNotMatch(guardSource, /valueType:/);
});

test("Guard contract runtime decoder is present", () => {
  const runtimePath = join(root, "src", "generated", "guard-contract-runtime.ts");
  assert.equal(existsSync(runtimePath), true);
});

test("Guard fixture is accepted by the runtime decoder", () => {
  const decoded = runtime.decodeGuardView(fixture);
  assert.equal(decoded.schemaVersion, 1);
  assert.equal(decoded.groups[0].params[0].valueType, "bool");
});

test("Guard decoder rejects unknown schema versions", () => {
  assert.throws(
    () => runtime.decodeGuardView({ ...fixture, schemaVersion: 999 }),
    (error) => error instanceof runtime.GuardContractError && /schemaVersion/.test(error.message),
  );
});

test("Guard decoder rejects unknown enum values", () => {
  const invalid = structuredClone(fixture);
  invalid.groups[0].params[0].status = "future_status";
  assert.throws(
    () => runtime.decodeGuardView(invalid),
    (error) => error instanceof runtime.GuardContractError && /unsupported value/.test(error.message),
  );
});
