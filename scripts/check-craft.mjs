#!/usr/bin/env node
/**
 * 壳 UI 视觉工艺规范的一致性门（docs/craft-spec.md §5）。
 * 规则改动必须同步规范正文；本脚本只读，不改任何文件。
 * 用法：node scripts/check-craft.mjs（挂在 pnpm test 首位）
 */
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const violations = [];
const fail = (rule, file, line, msg) => violations.push({ rule, file, line, msg });

function walk(dir, match, out = []) {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, match, out);
    else if (match(p)) out.push(p);
  }
  return out;
}

const rel = (p) => relative(root, p);
const tsxFiles = walk(join(root, "src"), (p) => p.endsWith(".tsx"));
const stylePath = join(root, "src/style.css");
const css = existsSync(stylePath) ? readFileSync(stylePath, "utf8") : "";

// ---- 规则 1：类串常量层已删除 ----
if (existsSync(join(root, "src/shared/lib/ui.ts"))) {
  fail("R1", "src/shared/lib/ui.ts", 0, "类串常量层必须已删除（调用点应收敛到组件原语）");
}

// ---- 规则 2：不得出现裸颜色字面量（例外须同行 craft-allow-color） ----
const COLOR_RE = /#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(|\boklch\(/;
const colorScanned = [...tsxFiles, ...(css ? [stylePath] : [])];
for (const p of colorScanned) {
  readFileSync(p, "utf8").split("\n").forEach((line, i) => {
    if (COLOR_RE.test(line) && !line.includes("craft-allow-color")) {
      fail("R2", rel(p), i + 1, line.trim().slice(0, 110));
    }
  });
}

// ---- 规则 3：@theme inline 必须覆盖上游原语引用的 token 族 ----
const themeStart = css.indexOf("@theme inline");
const themeEnd = themeStart < 0 ? -1 : css.indexOf("\n}", themeStart);
const themeBlock = themeStart < 0 ? "" : css.slice(themeStart, themeEnd);
const REQUIRED_BRIDGES = [
  "--shadow-2xs", "--shadow-xs", "--shadow-sm", "--shadow-md", "--shadow-lg", "--shadow-xl", "--shadow-2xl",
  "--radius-2xl", "--radius-3xl", "--radius-4xl", "--font-heading",
  "--color-sidebar:", "--color-chart-1:",
];
for (const token of REQUIRED_BRIDGES) {
  if (!themeBlock.includes(token)) {
    fail("R3", "src/style.css", 0, "@theme inline 缺少桥：" + token + "（不补会静默回落 Tailwind 默认值）");
  }
}

// ---- 规则 4：原语必须定义可见焦点态（非交互原语用 craft-allow-no-states 声明豁免） ----
for (const p of walk(join(root, "src/shared/components/ui"), (x) => x.endsWith(".tsx"))) {
  const src = readFileSync(p, "utf8");
  if (!src.includes("focus-visible:") && !src.includes("craft-allow-no-states")) {
    fail("R4", rel(p), 0, "缺少 focus-visible 焦点态，且未声明 craft-allow-no-states");
  }
}

// ---- 规则 5：不得再出现原生 <select ----
for (const p of tsxFiles) {
  readFileSync(p, "utf8").split("\n").forEach((line, i) => {
    if (/<select[\s>]/.test(line)) fail("R5", rel(p), i + 1, "原生 select 应改用 Select 原语");
  });
}

// ---- 规则 6：toast 队列与 .toast recipe 已删除（sonner 是唯一事实来源） ----
const storePath = join(root, "src/shared/store.ts");
if (existsSync(storePath) && /\btoasts\b/.test(readFileSync(storePath, "utf8"))) {
  fail("R6", "src/shared/store.ts", 0, "toast 队列状态应已删除（sonner 拥有队列）");
}
if (/^\.toast\b/m.test(css)) fail("R6", "src/style.css", 0, ".toast recipe 应已删除");

// ---- 报告 ----
if (violations.length === 0) {
  console.log("check-craft：通过（R1–R6 全部满足）");
  process.exit(0);
}
console.error("check-craft：发现 " + violations.length + " 处违规（规范见 docs/craft-spec.md）");
for (const v of violations) {
  console.error(`  [${v.rule}] ${v.file}${v.line ? ":" + v.line : ""} — ${v.msg}`);
}
process.exit(1);
