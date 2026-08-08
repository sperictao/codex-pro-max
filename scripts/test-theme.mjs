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

test("theme mode parser accepts only light, dark, and system", () => {
  assert.equal(getStoredTheme("light"), "light");
  assert.equal(getStoredTheme("dark"), "dark");
  assert.equal(getStoredTheme("system"), "system");
  assert.equal(getStoredTheme("solarized"), "system");
  assert.equal(getStoredTheme(null), "system");
});

test("theme family parser rejects invalid and prototype-chain localStorage values", () => {
  const families = { geist: { light: "geist-light", dark: "geist-dark", label: "Geist" } };
  assert.equal(getStoredFamily("geist", families), "geist");
  assert.equal(getStoredFamily("constructor", families), "geist");
  assert.equal(getStoredFamily("toString", families), "geist");
  assert.equal(getStoredFamily("missing", families), "geist");
});

test("dark resolution falls back to built-in dark when a family has no dark pairing", () => {
  const families = { custom: { light: "custom-light", label: "Custom" } };
  assert.equal(resolveDataTheme("dark", "custom", false, families), "dark");
  assert.equal(resolveDataTheme("system", "custom", true, families), "dark");
  assert.equal(resolveDataTheme("light", "custom", true, families), "custom-light");
});

test("system resolution follows the OS appearance for a paired family", () => {
  const families = { custom: { light: "custom-light", dark: "custom-dark", label: "Custom" } };
  assert.equal(resolveDataTheme("system", "custom", false, families), "custom-light");
  assert.equal(resolveDataTheme("system", "custom", true, families), "custom-dark");
  assert.equal(resolveDataTheme("dark", "custom", false, families), "custom-dark");
});
