#!/usr/bin/env node
// 发布前置校验（本地打 tag 前与 CI validate job 共用）：
//   必查：package.json / tauri.conf.json / Cargo.toml 三处版本号一致
//   --tag <vX.Y.Z>：追加校验 tag 与版本一致 + release-notes/<tag>.md 存在
// 用法：node scripts/check-release.mjs [--tag v0.12.2]
import { readFileSync, existsSync } from "node:fs";

const read = (p) => readFileSync(p, "utf8");

const pkgVersion = JSON.parse(read("package.json")).version;
const tauriVersion = JSON.parse(read("src-tauri/tauri.conf.json")).version;
const cargoVersion = read("src-tauri/Cargo.toml").match(/^version = "([^"]+)"$/m)?.[1];

const failures = [];
if (!cargoVersion) {
  failures.push("src-tauri/Cargo.toml 找不到 version 字段");
}
if (pkgVersion !== tauriVersion || tauriVersion !== cargoVersion) {
  failures.push(
    `版本号三处不一致：package.json=${pkgVersion} tauri.conf.json=${tauriVersion} Cargo.toml=${cargoVersion}`,
  );
}

const tagIdx = process.argv.indexOf("--tag");
const tag = tagIdx !== -1 ? process.argv[tagIdx + 1] : null;
if (tag) {
  if (tag !== `v${pkgVersion}`) {
    failures.push(`tag ${tag} 与版本号 v${pkgVersion} 不一致`);
  }
  const notesPath = `release-notes/${tag}.md`;
  if (!existsSync(notesPath)) {
    failures.push(`缺少 release notes：${notesPath}（build-release.yml 强制要求，见 AGENTS.md）`);
  }
}

if (failures.length > 0) {
  console.error(failures.map((f) => `✗ ${f}`).join("\n"));
  process.exit(1);
}
console.log(`✓ 发布校验通过：v${pkgVersion}${tag ? `（tag ${tag}）` : ""}`);
