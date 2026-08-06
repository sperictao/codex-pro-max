/**
 * Tauri updater 构建脚本。
 * 1. 生成生产 updater 配置
 * 2. 校验 updater 配置
 * 3. 构建 vendored taskboard 的 web UI（dist 被 gitignore，不构建则
 *    打包资源里没有静态页，注入到 Codex 的面板永远 404）
 * 4. 使用该配置作为 overlay 执行 tauri build
 *
 * 用法: node scripts/build-updater.mjs [--target <target>]
 */
import { spawnSync } from "node:child_process";
import { rmSync } from "node:fs";
import { pathToFileURL } from "node:url";

const rootDir = process.cwd();
const updaterConfigPath = "src-tauri/tauri.conf.updater.prod.json";

function fail(message) {
  console.error(`❌ ${message}`);
  process.exit(1);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: rootDir,
    encoding: "utf8",
    stdio: options.stdio ?? "inherit",
    env: options.env ?? process.env,
    shell: options.shell ?? false,
  });

  if (result.error) {
    fail(`执行 ${command} 失败：${result.error.message}`);
  }

  if (result.status !== 0) {
    fail(`执行 ${command} ${args.join(" ")} 失败：退出码 ${result.status ?? 1}`);
  }
}

function normalizeForwardedArgs(args = []) {
  return args.filter((arg) => arg !== "--");
}

function main(argv = process.argv.slice(2)) {
  // Step 1: 生成 updater 配置
  run(process.execPath, ["scripts/generate-updater-config.mjs"]);

  // Step 2: 校验 updater 配置
  run(process.execPath, ["scripts/validate-updater-config.mjs", updaterConfigPath]);

  // Step 3: 构建 vendored taskboard 的 web UI，产物落进打包资源
  run("npm", ["--prefix", "vendor/dashi-taskboard", "ci"], { shell: true });
  run("npm", ["--prefix", "vendor/dashi-taskboard", "run", "build:web"], { shell: true });
  // ponytail: node_modules 只是构建 dist 的中间产物，必须删掉——vendor 目录会被
  // resources 原样打包，带着它 RPM/deb 体积爆炸（CI 实测打包 14 分钟），
  // linuxdeploy 遍历 AppDir 逐文件跑 ldd，踩到 .bin 悬空符号链接直接失败
  rmSync("vendor/dashi-taskboard/node_modules", { recursive: true, force: true });

  // Step 4: 使用 overlay 配置执行 tauri build，透传额外参数
  // npm run tauri 已经是 tauri CLI 入口，不需要再传 tauri 子命令
  const tauriArgs = [
    "run-script",
    "tauri",
    "--",
    "build",
    "--config",
    updaterConfigPath,
    ...normalizeForwardedArgs(argv),
  ];

  // Windows 上 spawnSync 找不到 npm，需要 shell 模式
  run("npm", tauriArgs, { shell: true });
}

const isDirectExecution =
  process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url;

if (isDirectExecution) {
  main();
}
