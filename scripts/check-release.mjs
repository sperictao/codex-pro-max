#!/usr/bin/env node
// 发布前置校验（本地打 tag 前与 CI validate job 共用）：
//   必查：package.json / tauri.conf.json / Cargo.toml / Cargo.lock 四处版本一致
//   必查：Taskboard 运行时资源白名单完整，且固定 submodule 中预构建阶段应存在的资源真实存在
//         dist/web 与 node_modules 由 build:taskboard 在后续构建阶段生成，因此预检仅校验其映射存在
//   --tag <vX.Y.Z>：追加校验 tag 与版本一致 + release-notes/<tag>.md 存在
// 用法：node scripts/check-release.mjs [--tag v0.12.2]
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const read = (p) => readFileSync(p, "utf8");
const taskboardPackagePath = "vendor/dashi-taskboard/package.json";
if (!existsSync(taskboardPackagePath)) {
  try {
    execFileSync(
      "git",
      ["submodule", "update", "--init", "--recursive", "vendor/dashi-taskboard"],
      { stdio: "inherit" },
    );
  } catch (error) {
    console.error("✗ 无法初始化 vendor/dashi-taskboard submodule");
    process.exit(1);
  }
}

const pkgVersion = JSON.parse(read("package.json")).version;
const tauriConfPath = "src-tauri/tauri.conf.json";
const tauriConf = JSON.parse(read(tauriConfPath));
const tauriVersion = tauriConf.version;
const cargoVersion = read("src-tauri/Cargo.toml").match(/^version = "([^"]+)"$/m)?.[1];
const cargoLockVersion = read("src-tauri/Cargo.lock").match(
  /\[\[package\]\]\nname = "codex-pro-max"\nversion = "([^"]+)"/,
)?.[1];

const failures = [];
if (!cargoVersion) failures.push("src-tauri/Cargo.toml 找不到 version 字段");
if (!cargoLockVersion) failures.push("src-tauri/Cargo.lock 找不到 codex-pro-max 根包版本");
if (
  pkgVersion !== tauriVersion ||
  tauriVersion !== cargoVersion ||
  cargoVersion !== cargoLockVersion
) {
  failures.push(
    `版本号四处不一致：package.json=${pkgVersion} tauri.conf.json=${tauriVersion} Cargo.toml=${cargoVersion} Cargo.lock=${cargoLockVersion}`,
  );
}

const resources = tauriConf.bundle?.resources ?? {};
const resourceEntries =
  typeof resources === "object" && !Array.isArray(resources) ? Object.keys(resources) : resources;
const requiredTaskboardResources = [
  "../vendor/dashi-taskboard/server",
  "../vendor/dashi-taskboard/shared",
  "../vendor/dashi-taskboard/scripts",
  "../vendor/dashi-taskboard/cli",
  "../vendor/dashi-taskboard/inject",
  "../vendor/dashi-taskboard/skills",
  "../vendor/dashi-taskboard/dist/web",
  "../vendor/dashi-taskboard/package.json",
  "../vendor/dashi-taskboard/node_modules/smol-toml",
  "../vendor/dashi-taskboard/node_modules/ws",
  "../vendor/dashi-taskboard/integrations/deepseek-harness",
];
for (const required of requiredTaskboardResources) {
  if (!resourceEntries.includes(required)) {
    failures.push(`bundle.resources 缺少 Taskboard 运行时资源：${required}`);
  }
}

const tauriDir = dirname(tauriConfPath);
const generatedBeforeBuild = new Set([
  resolve(tauriDir, "../vendor/dashi-taskboard/dist/web"),
  resolve(tauriDir, "../vendor/dashi-taskboard/node_modules/smol-toml"),
  resolve(tauriDir, "../vendor/dashi-taskboard/node_modules/ws"),
]);
for (const src of resourceEntries) {
  const abs = resolve(tauriDir, src);
  if (generatedBeforeBuild.has(abs)) continue;
  if (!existsSync(abs)) {
    failures.push(`bundle.resources 源路径不存在：${src}（解析为 ${abs}）`);
  }
}

if (existsSync(taskboardPackagePath)) {
  const taskboardPkg = JSON.parse(read(taskboardPackagePath));
  if (taskboardPkg.engines?.node !== ">=22.5") {
    failures.push(
      `dashi-taskboard Node 引擎要求发生变化：当前=${taskboardPkg.engines?.node ?? "未声明"}，请同步 launcher 前置条件`,
    );
  }
}

const tagIdx = process.argv.indexOf("--tag");
const tag = tagIdx !== -1 ? process.argv[tagIdx + 1] : null;
if (tag) {
  if (tag !== `v${pkgVersion}`) failures.push(`tag ${tag} 与版本号 v${pkgVersion} 不一致`);
  const notesPath = `release-notes/${tag}.md`;
  if (!existsSync(notesPath)) {
    failures.push(`缺少 release notes：${notesPath}（build-release.yml 强制要求）`);
  }
}

if (failures.length > 0) {
  console.error(failures.map((f) => `✗ ${f}`).join("\n"));
  process.exit(1);
}
console.log(`✓ 发布校验通过：v${pkgVersion}${tag ? `（tag ${tag}）` : ""}`);
