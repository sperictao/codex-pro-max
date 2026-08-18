// pnpm tauri 薄封装：本机 Rust 工具链装在 ~/.cargo/bin（rustup 默认），但 GUI 环境/
// 某些 shell 的 PATH 可能不含它，导致 `cargo metadata` 找不到而启动失败。
// 这里把 cargo bin 目录前置进 PATH 再执行 tauri CLI（透传所有参数），窗口/vite 子进程随之继承。
// CI 走 build-updater.mjs（GitHub Actions 经 rust-toolchain action 保证 cargo 在 PATH），不受影响。

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);

// 追加 cargo bin（仅当存在；幂等：重复出现在 PATH 中无害）
const cargoBin = join(homedir(), ".cargo", "bin");
if (existsSync(cargoBin)) {
  process.env.PATH = `${cargoBin}${process.env.PATH ? `:${process.env.PATH}` : ""}`;
}

// node_modules/.bin/tauri 是 npm 装的可执行 shim。Windows 上实际可执行的是 tauri.CMD
// （cmd 批处理）；fileURLToPath 避免 pathname 在 Windows 产生 `/D:/` 前缀。
const binDir = fileURLToPath(new URL("../node_modules/.bin/", import.meta.url));
const isWin = process.platform === "win32";
const candidates = isWin
  ? [join(binDir, "tauri.CMD"), join(binDir, "tauri.exe"), join(binDir, "tauri.ps1")]
  : [join(binDir, "tauri")];
const cli = candidates.find((p) => existsSync(p));

if (!cli) {
  console.error(`✗ tauri CLI 未找到（${binDir}）`);
  process.exit(1);
}

const result = spawnSync(cli, args, {
  stdio: "inherit",
  env: process.env,
  // Windows 上 .CMD 是批处理，spawnSync 直接跑不了，需要 shell 解析
  shell: isWin,
});
if (result.error) {
  console.error(`✗ failed to run tauri: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 0);