/**
 * 校验 Tauri updater 生产配置文件。
 *
 * 用法: node scripts/validate-updater-config.mjs [path/to/tauri.conf.updater.prod.json]
 */
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const configPath = resolve(process.cwd(), process.argv[2] || "src-tauri/tauri.conf.updater.prod.json");

function fail(message) {
  console.error(`❌ ${message}`);
  process.exit(1);
}

if (!existsSync(configPath)) {
  fail(`未找到更新配置文件: ${configPath}`);
}

let config;
try {
  const raw = readFileSync(configPath, "utf8");
  config = JSON.parse(raw);
} catch (error) {
  fail(`读取或解析 JSON 失败: ${error.message}`);
}

const updater = config?.plugins?.updater;
if (!updater || typeof updater !== "object") {
  fail("缺少 plugins.updater 配置");
}

const pubkey = String(updater.pubkey || "").trim();
if (!pubkey || pubkey.includes("REPLACE_WITH")) {
  fail("updater.pubkey 为空或仍是占位值");
}

const endpoints = Array.isArray(updater.endpoints) ? updater.endpoints : [];
if (endpoints.length === 0) {
  fail("updater.endpoints 不能为空");
}

for (const endpoint of endpoints) {
  if (typeof endpoint !== "string" || endpoint.trim() === "") {
    fail("updater.endpoints 中存在空地址");
  }

  const value = endpoint.trim();
  if (!value.startsWith("https://")) {
    fail(`更新地址必须使用 https: ${value}`);
  }
}

if (config?.bundle?.createUpdaterArtifacts !== true) {
  fail("bundle.createUpdaterArtifacts 必须为 true");
}

console.log(`✅ Updater 配置校验通过: ${configPath}`);
