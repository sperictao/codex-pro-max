#!/usr/bin/env node
// 发布前置校验（本地打 tag 前与 CI validate job 共用）：
//   必查：package.json / tauri.conf.json / Cargo.toml 三处版本号一致
//   必查：tauri.conf.json bundle.resources 中被 git 跟踪的源路径真实存在
//         （v1.3.2 曾漏打包 skills/ 导致安装技能失败，见 AGENTS.md。
//         dist/web 等构建产物被 gitignore，CI validate 阶段尚未构建，
//         只对 git 已跟踪的路径做存在性校验）
//   --tag <vX.Y.Z>：追加校验 tag 与版本一致 + release-notes/<tag>.md 存在
// 用法：node scripts/check-release.mjs [--tag v0.12.2]
import { readFileSync, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { execFileSync } from "node:child_process";

const read = (p) => readFileSync(p, "utf8");

const pkgVersion = JSON.parse(read("package.json")).version;
const tauriConfPath = "src-tauri/tauri.conf.json";
const tauriConf = JSON.parse(read(tauriConfPath));
const tauriVersion = tauriConf.version;
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

// bundle.resources 的源路径（map 的 key）必须存在。../vendor/... 相对 tauri.conf.json 所在目录解析。
// dist/web 等构建产物被 gitignore，CI validate job 在构建前运行、产物尚不存在，
// 因此只对 git 已跟踪的路径做存在性校验（能拦下 skills/ 这种仓库内资源漏打包）。
const isGitTracked = (abs) => {
  try {
    execFileSync("git", ["ls-files", "--error-unmatch", abs], { stdio: "pipe" });
    return true;
  } catch {
    return false;
  }
};
const tauriDir = dirname(tauriConfPath);
const resources = tauriConf.bundle?.resources ?? {};
const resourceEntries =
  typeof resources === "object" && !Array.isArray(resources) ? Object.keys(resources) : resources;
for (const src of resourceEntries) {
  const abs = resolve(tauriDir, src);
  if (isGitTracked(abs) && !existsSync(abs)) {
    failures.push(`bundle.resources 源路径不存在（git 已跟踪）：${src}（解析为 ${abs}）`);
  }
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
