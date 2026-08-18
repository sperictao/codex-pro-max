// pnpm tauri 薄封装：本机 Rust 工具链装在 ~/.cargo/bin（rustup 默认），但 GUI 环境/
// 某些 shell 的 PATH 可能不含它，导致 `cargo metadata` 找不到而启动失败。
// 这里把 cargo bin 目录前置进 PATH 再执行 tauri CLI（透传所有参数），窗口/vite 子进程随之继承。
// CI 走 build-updater.mjs（GitHub Actions 经 rust-toolchain action 保证 cargo 在 PATH），不受影响。

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);

// 追加 cargo bin（仅当存在；幂等：重复出现在 PATH 中无害）
const cargoBin = join(homedir(), ".cargo", "bin");
if (existsSync(cargoBin)) {
  process.env.PATH = `${cargoBin}${process.env.PATH ? `:${process.env.PATH}` : ""}`;
}

// node_modules/.bin/tauri 是 npm 装的可执行 shim（shebang 脚本，跨平台由 npm 提供 bin 链接）
const cli = new URL("../node_modules/.bin/tauri", import.meta.url).pathname;
const result = spawnSync(cli, args, { stdio: "inherit", env: process.env });
if (result.error) {
  console.error(`✗ failed to run tauri: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 0);