import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { mkdtempSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { test } from "node:test";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const outputDir = mkdtempSync(join(tmpdir(), "dashi-theme-test-"));
process.on("exit", () => rmSync(outputDir, { recursive: true, force: true }));

// tsc 会跟随 import 把 theme-families.ts 一并编译进 outDir
execFileSync(process.execPath, [
  join(root, "node_modules/typescript/bin/tsc"),
  join(root, "src/theme.ts"),
  "--target",
  "ES2022",
  "--module",
  "CommonJS",
  "--moduleResolution",
  "node",
  "--strict",
  "--skipLibCheck",
  "--outDir",
  outputDir,
], { cwd: root, stdio: "inherit" });

const require = createRequire(import.meta.url);
const { getStoredFamily, getStoredTheme, resolveDataTheme } = require(join(outputDir, "theme.js"));
const { DEFAULT_FAMILY, THEME_FAMILIES } = require(join(outputDir, "theme-families.js"));

test("theme mode parser accepts only light, dark, and system", () => {
  assert.equal(getStoredTheme("light"), "light");
  assert.equal(getStoredTheme("dark"), "dark");
  assert.equal(getStoredTheme("system"), "system");
  assert.equal(getStoredTheme("solarized"), "system");
  assert.equal(getStoredTheme(null), "system");
});

test("family parser keeps manifest ids and rejects stale/prototype-chain values", () => {
  assert.equal(getStoredFamily("catppuccin"), "catppuccin");
  assert.equal(getStoredFamily("geist"), DEFAULT_FAMILY); // 旧版族名静默回落
  assert.equal(getStoredFamily("constructor"), DEFAULT_FAMILY);
  assert.equal(getStoredFamily("__proto__"), DEFAULT_FAMILY);
  assert.equal(getStoredFamily("toString"), DEFAULT_FAMILY);
  assert.equal(getStoredFamily(null), DEFAULT_FAMILY);
});

test("resolveDataTheme follows the <family>-light|dark naming convention", () => {
  assert.equal(resolveDataTheme("light", "claude", true), "claude-light");
  assert.equal(resolveDataTheme("dark", "claude", false), "claude-dark");
  assert.equal(resolveDataTheme("system", "claude", true), "claude-dark");
  assert.equal(resolveDataTheme("system", "claude", false), "claude-light");
});

test("resolveDataTheme falls back to the default family for unknown ids", () => {
  assert.equal(resolveDataTheme("dark", "missing", false), `${DEFAULT_FAMILY}-dark`);
  assert.equal(resolveDataTheme("system", "missing", false), `${DEFAULT_FAMILY}-light`);
});

test("manifest sanity: full preset set with the default family present", () => {
  assert.equal(THEME_FAMILIES.length, 42);
  assert.ok(THEME_FAMILIES.some((f) => f.id === DEFAULT_FAMILY));
  assert.ok(THEME_FAMILIES.every((f) => f.id && f.label));
});
