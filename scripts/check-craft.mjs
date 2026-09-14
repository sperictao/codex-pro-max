#!/usr/bin/env node
/**
 * 壳 UI 视觉工艺规范的一致性门（docs/craft-spec.md §5）。
 * 规则改动必须同步规范正文；本脚本只读，不改任何文件。
 * 用法：node scripts/check-craft.mjs（挂在 pnpm test 首位）
 */
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { basename, dirname, join, relative } from "node:path";
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
const srcDir = join(root, "src");
const uiDir = join(srcDir, "shared/components/ui");
const sourceFiles = walk(srcDir, (p) => p.endsWith(".ts") || p.endsWith(".tsx"));
const tsxFiles = sourceFiles.filter((p) => p.endsWith(".tsx"));
const uiFiles = walk(uiDir, (p) => p.endsWith(".tsx"));
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
  "--color-sidebar:", "--color-sidebar-foreground:", "--color-sidebar-primary:",
  "--color-sidebar-primary-foreground:", "--color-sidebar-accent:", "--color-sidebar-accent-foreground:",
  "--color-sidebar-border:", "--color-sidebar-ring:",
  "--color-chart-1:", "--color-chart-2:", "--color-chart-3:", "--color-chart-4:", "--color-chart-5:",
];
for (const token of REQUIRED_BRIDGES) {
  if (!themeBlock.includes(token)) {
    fail("R3", "src/style.css", 0, "@theme inline 缺少桥：" + token + "（不补会静默回落 Tailwind 默认值）");
  }
}

// ---- 规则 4：关键交互原语按自身交互模型检查状态契约 ----
// 不再使用“文件里出现一次 focus-visible 或写一个豁免注释就算通过”的文件级捷径。
// 原生表单控件检查 focus/invalid/disabled；状态型控件检查 checked/unchecked；
// roving-focus 组件检查 selection/focus + disabled。新增关键交互原语时应在这里显式登记契约。
const STATE_CONTRACTS = {
  "button.tsx": {
    focus: ["focus-visible:border-ring", "focus-visible:ring-3"],
    hover: ["hover:"],
    pressed: ["active:"],
    disabled: ["disabled:pointer-events-none", "disabled:opacity-50"],
  },
  "input.tsx": {
    focus: ["focus-visible:border-ring", "focus-visible:ring-3"],
    invalid: ["aria-invalid:border-destructive", "aria-invalid:ring-3"],
    disabled: ["disabled:opacity-50"],
  },
  "textarea.tsx": {
    focus: ["focus-visible:border-ring", "focus-visible:ring-3"],
    invalid: ["aria-invalid:border-destructive", "aria-invalid:ring-3"],
    disabled: ["disabled:opacity-50"],
  },
  "select.tsx": {
    triggerFocus: ["focus-visible:border-ring", "focus-visible:ring-3"],
    triggerInvalid: ["aria-invalid:border-destructive", "aria-invalid:ring-3"],
    disabled: ["disabled:opacity-50", "data-disabled:opacity-50"],
    itemFocus: ["focus:bg-accent"],
  },
  "switch.tsx": {
    focus: ["focus-visible:border-ring", "focus-visible:ring-3"],
    checked: ["data-checked:bg-primary"],
    unchecked: ["data-unchecked:bg-input"],
    disabled: ["data-disabled:opacity-50"],
  },
  "command.tsx": {
    selected: ["data-selected:bg-muted"],
    disabled: ["data-[disabled=true]:pointer-events-none", "data-[disabled=true]:opacity-50"],
  },
  "dropdown-menu.tsx": {
    itemFocus: ["focus:bg-accent"],
    disabled: ["data-disabled:pointer-events-none", "data-disabled:opacity-50"],
  },
};
for (const [file, contract] of Object.entries(STATE_CONTRACTS)) {
  const p = join(uiDir, file);
  if (!existsSync(p)) {
    fail("R4", rel(p), 0, "交互原语契约已登记但文件不存在");
    continue;
  }
  const src = readFileSync(p, "utf8");
  for (const [state, needles] of Object.entries(contract)) {
    const missing = needles.filter((needle) => !src.includes(needle));
    if (missing.length > 0) {
      fail("R4", rel(p), 0, `${state} 状态契约缺少：${missing.join(", ")}`);
    }
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

// ---- 规则 7：原语必须有生产 UI 层中的真实调用点，禁止预置零调用点组件 ----
const consumerSources = sourceFiles
  .filter((p) => {
    const path = rel(p).replaceAll("\\", "/");
    return !path.startsWith("src/shared/components/ui/") && !path.startsWith("src/smoke/") && !path.includes(".test.");
  })
  .map((p) => readFileSync(p, "utf8"));
for (const p of uiFiles) {
  const moduleName = basename(p, ".tsx");
  const importPath = `@/shared/components/ui/${moduleName}`;
  if (!consumerSources.some((src) => src.includes(importPath))) {
    fail("R7", rel(p), 0, "原语没有生产 UI 层中的真实调用点；按需引入，不预先囤积");
  }
}

// ---- 报告 ----
if (violations.length === 0) {
  console.log("check-craft：通过（R1–R7 全部满足）");
  process.exit(0);
}
console.error("check-craft：发现 " + violations.length + " 处违规（规范见 docs/craft-spec.md）");
for (const v of violations) {
  console.error(`  [${v.rule}] ${v.file}${v.line ? ":" + v.line : ""} — ${v.msg}`);
}
process.exit(1);
