/**
 * 从 updater-assets 目录中的构建产物生成 Tauri updater manifest (latest.json)。
 *
 * 用法:
 *   node scripts/generate-latest-json.mjs \
 *     --input ./updater-assets \
 *     --output ./updater-assets/latest.json \
 *     --version 0.1.0 \
 *     --baseUrl https://github.com/owner/repo/releases/download/v0.1.0 \
 *     --notes-file ./release-notes/v0.1.0.md
 */
import { readFileSync, writeFileSync, readdirSync, existsSync } from "node:fs";
import { resolve, basename } from "node:path";

function parseArgs() {
  const args = {};
  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--input") args.input = argv[++i];
    else if (argv[i] === "--output") args.output = argv[++i];
    else if (argv[i] === "--version") args.version = argv[++i];
    else if (argv[i] === "--baseUrl") args.baseUrl = argv[++i];
    else if (argv[i] === "--notes-file") args.notesFile = argv[++i];
  }
  if (!args.input || !args.output || !args.version || !args.baseUrl) {
    console.error("Missing required arguments. See usage in script header.");
    process.exit(1);
  }
  return args;
}

function findFile(dir, pattern) {
  const files = readdirSync(dir);
  const matches = files.filter((f) => {
    if (typeof pattern === "string") return f.includes(pattern);
    return pattern.test(f);
  });
  return matches.length > 0 ? resolve(dir, matches[0]) : null;
}

function readSignature(dir, assetName) {
  const sigPath = resolve(dir, assetName + ".sig");
  if (!existsSync(sigPath)) return null;
  return readFileSync(sigPath, "utf8").trim();
}

// GitHub 上传 release 资产时会把空格等不安全字符替换成 "."，
// latest.json 里的 URL 必须用净化后的资产名，否则下载 404。
const ghAssetName = (name) => name.replace(/[^a-zA-Z0-9._-]/g, ".");

function main() {
  const args = parseArgs();
  const inputDir = resolve(args.input);
  const files = readdirSync(inputDir);

  const notes = args.notesFile && existsSync(args.notesFile)
    ? readFileSync(args.notesFile, "utf8")
    : "";

  const manifest = {
    version: args.version,
    notes,
    pub_date: new Date().toISOString(),
    platforms: {},
  };

  // macOS aarch64
  const macArmTar = files.find((f) => f.includes("aarch64") && f.endsWith(".app.tar.gz") && !f.endsWith(".sig"));
  if (macArmTar) {
    const sig = readSignature(inputDir, macArmTar);
    manifest.platforms["darwin-aarch64"] = {
      signature: sig || "",
      url: `${args.baseUrl}/${ghAssetName(macArmTar)}`,
    };
  }

  // macOS x86_64
  const macX64Tar = files.find((f) => f.includes("x64") && f.endsWith(".app.tar.gz") && !f.endsWith(".sig"));
  if (macX64Tar) {
    const sig = readSignature(inputDir, macX64Tar);
    manifest.platforms["darwin-x86_64"] = {
      signature: sig || "",
      url: `${args.baseUrl}/${ghAssetName(macX64Tar)}`,
    };
  }

  // Windows x86_64
  const winExe = files.find((f) => f.endsWith("-setup.exe") && !f.endsWith(".sig"));
  if (winExe) {
    const sig = readSignature(inputDir, winExe);
    manifest.platforms["windows-x86_64"] = {
      signature: sig || "",
      url: `${args.baseUrl}/${ghAssetName(winExe)}`,
    };
  }

  // Linux x86_64
  const linuxAppImage = files.find((f) => f.endsWith(".AppImage") && !f.endsWith(".sig"));
  if (linuxAppImage) {
    const sig = readSignature(inputDir, linuxAppImage);
    manifest.platforms["linux-x86_64"] = {
      signature: sig || "",
      url: `${args.baseUrl}/${ghAssetName(linuxAppImage)}`,
    };
  }

  const platformCount = Object.keys(manifest.platforms).length;
  if (platformCount === 0) {
    console.error("No platform assets found in", inputDir);
    process.exit(1);
  }

  writeFileSync(args.output, JSON.stringify(manifest, null, 2) + "\n", "utf8");
  console.log(`Generated ${args.output} with ${platformCount} platforms`);
}

main();
