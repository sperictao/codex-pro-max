import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const generated = join(root, "src", "generated", "guard-contracts.ts");
assert.equal(existsSync(generated), true, "checked-in Guard contract is missing");

const tempDir = mkdtempSync(join(tmpdir(), "dashi-guard-contracts-"));
const output = join(tempDir, "guard-contracts.ts");
try {
  const result = spawnSync(
    process.env.CARGO ?? "cargo",
    [
      "run",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--locked",
      "--",
      "--generate-guard-contracts",
      output,
    ],
    { cwd: root, stdio: "inherit" },
  );
  assert.equal(result.status, 0, "Rust contract generator failed");
  assert.equal(
    Buffer.compare(readFileSync(generated), readFileSync(output)),
    0,
    "checked-in Guard contract differs from Rust-generated output; run npm run check:contracts -- --fix is not supported, inspect the diff and regenerate intentionally",
  );
} finally {
  rmSync(tempDir, { recursive: true, force: true });
}
