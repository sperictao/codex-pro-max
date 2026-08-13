/**
 * 生成生产环境的 Tauri updater 配置文件（tauri.conf.updater.prod.json）。
 *
 * 从环境变量读取 updater 公钥和签名密钥，生成 Tauri CLI --config overlay 配置。
 * 该文件被 .gitignore 忽略，仅在构建时生成。
 */
import { writeFileSync, existsSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, "..");
const outputPath = resolve(projectRoot, "src-tauri", "tauri.conf.updater.prod.json");

const pubkey = process.env.TAURI_UPDATER_PUBKEY;
if (!pubkey || pubkey.includes("REPLACE_WITH")) {
  console.error("TAURI_UPDATER_PUBKEY 环境变量未设置或无效");
  process.exit(1);
}

const repoSlug = process.env.TAURI_UPDATER_REPO || "sperictao/codex-pro-max";
const endpoint =
  process.env.TAURI_UPDATER_ENDPOINT ||
  `https://github.com/${repoSlug}/releases/latest/download/latest.json`;

const config = {
  bundle: {
    createUpdaterArtifacts: true,
  },
  plugins: {
    updater: {
      pubkey,
      endpoints: [endpoint],
    },
  },
};

writeFileSync(outputPath, JSON.stringify(config, null, 2) + "\n", "utf8");
console.log(`Updater 配置已生成: ${outputPath}`);
console.log(`Endpoint: ${endpoint}`);
