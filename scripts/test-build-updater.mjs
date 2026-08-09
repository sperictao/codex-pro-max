import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const buildScript = fileURLToPath(new URL("./build-updater.mjs", import.meta.url));

test("updater build always passes Cargo --locked after the Tauri separator", () => {
  const fixtureRoot = mkdtempSync(join(tmpdir(), "dashi-build-updater-"));
  const binDir = join(fixtureRoot, "bin");
  const logPath = join(fixtureRoot, "npm-argv.jsonl");

  try {
    mkdirSync(join(fixtureRoot, "scripts"), { recursive: true });
    mkdirSync(join(fixtureRoot, "vendor", "dashi-taskboard"), { recursive: true });
    mkdirSync(binDir, { recursive: true });
    writeFileSync(join(fixtureRoot, "scripts", "generate-updater-config.mjs"), "");
    writeFileSync(join(fixtureRoot, "scripts", "validate-updater-config.mjs"), "");

    const fakeNpmScript = join(binDir, "fake-npm.cjs");
    writeFileSync(
      fakeNpmScript,
      `const fs = require("node:fs");\nfs.appendFileSync(process.env.DASHI_NPM_ARGV_LOG, JSON.stringify(process.argv.slice(2)) + "\\n");\n`,
    );

    const fakeNpmPath = join(binDir, "npm");
    writeFileSync(fakeNpmPath, `#!/usr/bin/env node\nrequire("./fake-npm.cjs");\n`);
    chmodSync(fakeNpmPath, 0o755);
    writeFileSync(
      join(binDir, "npm.cmd"),
      `@echo off\r\n"${process.execPath}" "${fakeNpmScript}" %*\r\n`,
    );

    const result = spawnSync(
      process.execPath,
      [buildScript, "--target", "aarch64-apple-darwin"],
      {
        cwd: fixtureRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          DASHI_NPM_ARGV_LOG: logPath,
          PATH: `${binDir}${delimiter}${process.env.PATH ?? ""}`,
        },
      },
    );

    assert.equal(result.status, 0, result.stderr || result.stdout);
    const calls = readFileSync(logPath, "utf8")
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line));

    assert.deepEqual(calls.at(-1), [
      "run-script",
      "tauri",
      "--",
      "build",
      "--config",
      "src-tauri/tauri.conf.updater.prod.json",
      "--target",
      "aarch64-apple-darwin",
      "--",
      "--locked",
    ]);
  } finally {
    rmSync(fixtureRoot, { recursive: true, force: true });
  }
});
