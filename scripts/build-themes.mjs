#!/usr/bin/env node
/**
 * 主题构建（ADR 0008）：从 tweakcn registry 拉取预设并本地化。
 *
 * 产出（全部提交进 git，勿手改）：
 *   src/themes.css        — [data-theme="<id>-light|dark"] scoped token 块 + @font-face
 *   src/theme-families.ts — 族清单 manifest（主题选择器数据源）
 *   assets/fonts/*.woff2  — 预设引用的 Google 字体（仅 latin / latin-ext 子集，离线可用）
 *
 * 重跑本脚本 = 主动跟随上游。PRESET_IDS 写死：上游新增预设不自动进入。
 * 用法：node scripts/build-themes.mjs（需要网络）
 */
import { mkdir, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const REGISTRY = "https://tweakcn.com/r/themes";

// tweakcn 预设全集（https://github.com/jnsahaj/tweakcn utils/theme-presets.ts）
const PRESET_IDS = [
  "modern-minimal", "violet-bloom", "t3-chat", "twitter", "mocha-mousse",
  "bubblegum", "amethyst-haze", "notebook", "doom-64", "catppuccin",
  "graphite", "perpetuity", "kodama-grove", "cosmic-night", "tangerine",
  "quantum-rose", "nature", "bold-tech", "elegant-luxury", "amber-minimal",
  "supabase", "neo-brutalism", "solar-dusk", "claymorphism", "cyberpunk",
  "pastel-dreams", "clean-slate", "caffeine", "ocean-breeze", "retro-arcade",
  "midnight-bloom", "candyland", "northern-lights", "vintage-paper",
  "sunset-horizon", "starry-night", "claude", "vercel", "darkmatter",
  "mono", "soft-pop", "sage-garden",
];

// 字体栈首项是这些时不下载（系统/通用字体）
const SYSTEM_FONTS = new Set([
  "georgia", "courier new", "arial", "helvetica", "times new roman",
  "system-ui", "ui-monospace", "ui-sans-serif", "sf mono", "menlo",
  "monospace", "sans-serif", "serif", "cursive",
]);

const FONT_SUBSETS = new Set(["latin", "latin-ext"]);
// 拿 woff2 需要现代浏览器 UA
const CHROME_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

async function fetchText(url, headers = {}) {
  const res = await fetch(url, { headers, signal: AbortSignal.timeout(30_000) });
  if (!res.ok) throw new Error(`GET ${url} -> ${res.status}`);
  return res.text();
}

async function fetchJson(url) {
  return JSON.parse(await fetchText(url));
}

function firstFont(stack) {
  return stack.split(",")[0].trim().replace(/^['"]|['"]$/g, "");
}

function slug(name) {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}

/** Google Fonts css2：优先可变字重，失败回退静态字重，再退裸族名 */
async function fontFacesCss(family) {
  const enc = family.trim().replace(/\s+/g, "+");
  const attempts = [
    `https://fonts.googleapis.com/css2?family=${enc}:wght@100..900&display=swap`,
    `https://fonts.googleapis.com/css2?family=${enc}:wght@400;500;600;700&display=swap`,
    `https://fonts.googleapis.com/css2?family=${enc}&display=swap`,
  ];
  for (const url of attempts) {
    try {
      return await fetchText(url, { "user-agent": CHROME_UA });
    } catch { /* 试下一档 */ }
  }
  throw new Error(`无法获取字体 CSS：${family}`);
}

/** 解析 css2 响应，只保留 latin / latin-ext 子集 */
function parseFontFaces(css) {
  const faces = [];
  const re = /\/\* ([a-z-]+) \*\/\s*@font-face \{([^}]+)\}/g;
  let m;
  while ((m = re.exec(css)) !== null) {
    const [, subset, body] = m;
    if (!FONT_SUBSETS.has(subset)) continue;
    const weight = body.match(/font-weight:\s*([^;]+);/)?.[1].trim() ?? "400";
    const style = body.match(/font-style:\s*([^;]+);/)?.[1].trim() ?? "normal";
    const src = body.match(/url\((https:[^)]+)\)/)?.[1];
    const range = body.match(/unicode-range:\s*([^;]+);/)?.[1].trim() ?? "";
    const family = body.match(/font-family:\s*'([^']+)'/)?.[1];
    if (src && family && style === "normal") faces.push({ family, subset, weight, style, src, range });
  }
  return faces;
}

async function main() {
  console.log(`拉取 ${PRESET_IDS.length} 个 tweakcn 预设…`);
  const presets = [];
  for (const id of PRESET_IDS) {
    const item = await fetchJson(`${REGISTRY}/${id}`);
    if (!item.cssVars?.light || !item.cssVars?.dark || !item.cssVars?.theme) {
      throw new Error(`预设 ${id} 的 registry 响应缺 cssVars（light/dark/theme）`);
    }
    presets.push({ id, name: item.name ?? id, cssVars: item.cssVars });
    process.stdout.write(".");
  }
  console.log(" 完成");

  // ---- 字体：收集全部栈首项，去重，下载 woff2，生成 @font-face ----
  const fontFamilies = new Set();
  for (const p of presets) {
    for (const key of ["font-sans", "font-serif", "font-mono"]) {
      const stack = p.cssVars.theme[key];
      if (!stack) continue;
      const first = firstFont(stack);
      if (!SYSTEM_FONTS.has(first.toLowerCase())) fontFamilies.add(first);
    }
  }
  console.log(`下载 ${fontFamilies.size} 种字体（latin / latin-ext）…`);

  const fontsDir = join(root, "assets", "fonts");
  await rm(fontsDir, { recursive: true, force: true });
  await mkdir(fontsDir, { recursive: true });

  const fontFaceCss = [];
  for (const family of [...fontFamilies].sort()) {
    let faces;
    try {
      faces = parseFontFaces(await fontFacesCss(family));
    } catch {
      faces = [];
    }
    if (faces.length === 0) {
      console.warn(`\n  ! ${family}：Google Fonts 无此字体，跳过（运行时回落字体栈）`);
      continue;
    }
    for (const face of faces) {
      const file = `${slug(face.family)}-${face.subset}-${face.weight.replace(/\s+/g, "")}.woff2`;
      const buf = Buffer.from(
        await (await fetch(face.src, { headers: { "user-agent": CHROME_UA }, signal: AbortSignal.timeout(30_000) })).arrayBuffer(),
      );
      await writeFile(join(fontsDir, file), buf);
      fontFaceCss.push(`@font-face {
  font-family: '${face.family}';
  font-style: normal;
  font-weight: ${face.weight};
  font-display: swap;
  src: url("../assets/fonts/${file}") format("woff2");
  unicode-range: ${face.range};
}`);
    }
    process.stdout.write(".");
  }
  console.log(" 完成");

  // ---- themes.css ----
  const varBlock = (vars) =>
    Object.entries(vars)
      .map(([k, v]) => `  --${k}: ${v};`)
      .join("\n");

  const themeBlocks = presets.flatMap((p) => {
    const shared = varBlock(p.cssVars.theme);
    return [
      `[data-theme="${p.id}-light"] {\n  color-scheme: light;\n${shared}\n${varBlock(p.cssVars.light)}\n}`,
      `[data-theme="${p.id}-dark"] {\n  color-scheme: dark;\n${shared}\n${varBlock(p.cssVars.dark)}\n}`,
    ];
  });

  const themesCss = `/* 生成物：scripts/build-themes.mjs 产出，勿手改。
   token 上游：https://tweakcn.com registry（${PRESET_IDS.length} 个预设）
   重新生成：node scripts/build-themes.mjs（需要网络） */

${fontFaceCss.join("\n\n")}

${themeBlocks.join("\n\n")}
`;
  await writeFile(join(root, "src", "themes.css"), themesCss);

  // ---- theme-families.ts ----
  const manifest = `// 生成物：scripts/build-themes.mjs 产出，勿手改
export interface ThemeFamilyInfo {
  id: string;
  label: string;
}

export const DEFAULT_FAMILY = "vercel";

export const THEME_FAMILIES: readonly ThemeFamilyInfo[] = [
${presets.map((p) => `  { id: ${JSON.stringify(p.id)}, label: ${JSON.stringify(p.name)} },`).join("\n")}
];
`;
  await writeFile(join(root, "src", "theme-families.ts"), manifest);

  console.log(`完成：${presets.length} 族（${presets.length * 2} 个 token 块），${fontFaceCss.length} 个 @font-face`);
}

main().catch((e) => {
  console.error(`build-themes 失败：${e.message}`);
  process.exit(1);
});
