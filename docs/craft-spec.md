# 壳 UI 视觉工艺规范（Craft Spec）

壳 UI 的组件与页面"看起来是否统一、是否精致"的**唯一成文判定来源**。新增屏与组件必须匹配本规范；`scripts/check-craft.mjs` 是它的可执行门，挂在 `pnpm test` 上。

适用范围：启动器壳（`src/`）。**不含** `vendor/dashi-taskboard`——那是独立应用（见 [CONTEXT.md](../CONTEXT.md) 与 [ADR 0002](adr/0002-taskboard-submodule-packaging.md)）。

配方来源：`arhamkhnz/next-shadcn-admin-dashboard` @ `3858591`（MIT，shadcn `radix-nova` 风格）的**一次性移植**。不跟随其版本演进；颜色资产同源于 tweakcn（[ADR 0008](adr/0008-tweakcn-token-theming.md)），不引入第二个上游。

## 1. 十条规约

| # | 规约 | 落法 |
| --- | --- | --- |
| R1 | **浮层身份是描边，不是阴影** | 浮层用 `ring-1 ring-foreground/10`（取自 `--foreground`，对 42 族自动成立）；Card 本体无阴影无边框 |
| R2 | **阴影走语义阶梯** | 静止内容卡 `shadow-xs` → 嵌套 chrome / 活动页签 `shadow-sm` → 临时弹层 `shadow-md`（必配 R1 描边）→ 面板 `shadow-lg` → 仅提示类 `shadow-xl`；`shadow-none` 是显式重置，不是装饰 |
| R3 | **每组件一个密度变量** | Card 用 `[--card-spacing:--spacing(4)]`，全部内边距读 `px-(--card-spacing)`；`size=sm` 只改写该变量为 `--spacing(3)` |
| R4 | **状态四元组逐字统一** | `outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50` + `aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20` + `disabled:pointer-events-none disabled:opacity-50` + 暗色镜像；高对比度下再补 1px `outline-ring` |
| R5 | **按下反馈** | `active:translate-y-px`；弹层触发器用 `active:not-aria-[haspopup]:translate-y-px` 抑制，避免菜单开启时抖动 |
| R6 | **半径夹取** | 小控件 `rounded-[min(var(--radius-md),10px)]`（xs）/ `12px`（sm）；药丸徽章 `rounded-4xl`（= `calc(var(--radius) + 16px)`）；复选框用 `rounded-[4px]` 而非半径 token |
| R7 | **命中区扩展** | 视觉尺寸 < 24px 的控件补 `after:absolute after:-inset-x-3 after:-inset-y-2`（16px 控件 → ~40px 触摸目标） |
| R8 | **密度带 4/8/16** | 间距只用 `0.5 / 1 / 1.5 / 2 / 3 / 4 / 5 / 6`，以 4/8/16px 为主，避免 12/20/24；页节奏 `gap-4 md:gap-6`；壳内边距 `p-4 md:p-6`；顶栏高度写成 `--header-height` token |
| R9 | **动效预算** | 弹层 100ms / 面板 200ms；进场 `animate-in fade-in-0 zoom-in-95` + 从触发器 origin + 方向性 slide；遮罩 `bg-black/10 supports-backdrop-filter:backdrop-blur-xs`；**主题/字体切换期间挂 `.disable-transitions`** |
| R10 | **数字与微标签** | 一切数值 `tabular-nums`（KPI 为 `text-3xl leading-none tracking-tight`）；微标签 `text-[10px]/[11px] uppercase tracking-widest text-muted-foreground`；图标默认 `size-4` 且写成可覆盖的 `[&_svg:not([class*='size-'])]:size-4` |

### 1.1 步骤 1..6 落地的固定约定

- **破坏性行操作一律进 `⋯` DropdownMenu**，主操作（Lock/Unlock、Edit、Apply…）留在行内。触发器用 `<Button variant="ghost" size="icon-sm" aria-label={t("More actions")}>` + `MoreHorizontalIcon`，内容用 `<DropdownMenuContent align="end">`。
- **帮助说明一律用 Tooltip 原语**（不再手写绝对定位的说明卡）。`TooltipProvider` 就近包裹，保证组件脱离 App 也能独立渲染（测试会单独挂载视图）。
- **内容卡阴影由调用点加 `shadow-xs`**：Card 原语故意不带阴影；嵌套的下级 chrome 用 `shadow-sm`；卡内行/胶囊不加阴影。
- **顶栏高度走 `--header-height` token**（`h-(--header-height)`），不再散落 `py-2.5`。
- **导航项用 Button 原语**：`variant={active ? "secondary" : "ghost"} size="sm"` + `aria-current="page"`；`.header-btn` / `.nav-item` recipe 已删除，不得复活。
- **可交互 recipe 必须有键盘焦点态**：`.select-card` 这类自绘控件要补 `focus-visible` 环（R4 对它们同样成立）。

## 2. 档位表

- **字号**：`text-xs` / `text-sm`（正文默认）/ `text-base`（标题、移动端输入）/ `text-[0.8rem]`（按钮 sm）/ `text-3xl`（KPI 数值）。**不新增 `--text-*` / `--leading-*` / `--font-weight-*` token**：一致性来自"只用这几档"，上游同样不做 type token。
- **字距**：只用已有 tracking token 与 `tracking-tight`（标题）/ `tracking-widest`（微标签）两种用法，不写任意值。
- **半径**：`rounded-sm…4xl` 全部由 `--radius` 阶梯派生，桥必须完整（见 §4 步骤 0）。
- **间距**：见 R8。
- **遮罩**：`bg-black/10` + `backdrop-blur-xs`，两模式同值。
- **图标**：默认 `size-4`；Button xs/sm 收窄为 `size-3 / size-3.5`；Badge 用 `size-3!`。

## 3. 原语清单

批 0 为今天即有真实调用点者，随"横切"一次引入；其余**按调用点引入并在同一次提交里接线**（不预先囤积零调用点的原语）。

| 原语 | 今天调用点 | 依赖 | 批次 |
| --- | --- | --- | --- |
| Button（含 ButtonGroup） | 49 个 `<button>` + 类串常量 7 个 | cva | 0 |
| Input / Textarea | 31 | — | 0 |
| Select | 7 个原生 `<select>` | radix-ui | 0 |
| Switch | 8（7× 假开关 + 1× `.text-switch`） | radix-ui | 0 |
| Card | 10 处重复卡片块 | — | 0 |
| Dialog | `Modal` 壳 + 6 处调用 | radix-ui | 0 |
| Badge | 状态徽章 4 文件 + Active 标签 | — | 0 |
| Tooltip | 看守参数说明悬浮卡 | radix-ui | 0 |
| Separator | 顶栏竖分隔 | — | 0 |
| DropdownMenu | 0 | radix-ui | ✅ 已随首屏落地：看守参数行/文件行与模型供应商/预设行的 **Delete** 收进 `⋯`，主操作（Lock/Unlock、Edit、Apply）仍在行内 |
| Table | 0 | — | 随首屏（看守参数列表 / 模型供应商列表密度重做） |
| Command（cmdk） | 0 | cmdk | **单独立项**：需要真实入口（Cmd+K 面板）才算功能，否则是死代码 |

| Toast（sonner） | store 的 toast 队列 + `Toaster.tsx` + `.toast` recipe | sonner | 0 |

**胶囊约定**：状态/版本胶囊一律 `<Badge variant="secondary">`，**不允许调用点自写着色**（原 `bg-primary/15 text-primary` 那类着色已统一移除）；需要等宽数字时加 `font-mono`。

**不引入**：Checkbox（全仓 checkbox 均作开关用，0 真实调用点）；`cn` npm 包（已有 `clsx + tailwind-merge`）。

**sonner 落法**：`Toaster.tsx` 改为 sonner 的薄包装，位置右下（与现行为一致），按上游配方重贴主题——`--normal-bg: var(--popover)`、`--normal-text: var(--popover-foreground)`、`--normal-border: var(--border)`、`--border-radius: var(--radius)`，图标强制 `size-4`。store 的 toast 队列状态（`toasts` / `dismissToast` / `ToastItem` / `ToastType`）与 `style.css` 的 `.toast` recipe **删除**，调用点直连 `toast.success/error/info`——队列若保留就是与 sonner 各自持有定时器（我们 3.3s、sonner 默认 4s），属于第二份真相。

## 4. 改造顺序

**步骤 0 — 横切 ✅ 已完成并验证**（`check-craft` 通过 / `tsc` 0 错 / `pnpm test` 绿 / 冒烟 92-92 前后一致 / 生产构建通过）

1. `@theme inline` 补齐桥：`shadow-2xs…2xl`、`radius-2xl/3xl/4xl`、`font-heading`、`color-sidebar*`、`color-chart*`。**这是移植的前置条件**——不补桥，原语里的 shadow/radius 工具类会静默回落到 Tailwind 默认值而不跟随主题族。
2. 依赖：`radix-ui`（统一包）、`class-variance-authority`、`lucide-react`、`tw-animate-css`、`sonner`。
3. 落批 0 原语源码进 `src/shared/components/ui/`。
4. 全局项：`.disable-transitions`（主题/字体切换期间挂载）、`overscroll-behavior: none`。
5. 删除 `src/shared/lib/ui.ts`，12 个视图的 85 个调用点改走原语（本步骤最大的机械改动）。
6. 本规范落文 + `scripts/check-craft.mjs` 上线并挂进 `pnpm test`。
7. 修正 `components.json`：`style` 由 `new-york` 改为 `radix-nova`（`ui` alias 与 `iconLibrary` 从此名副其实）。
**移植适配（必读，改动原语或新增原语前先看这里）**

- **暗面变体**：本项目的暗色是 `<html data-theme="<族id>-dark">`，不是 `.dark` 类。`src/style.css` 必须保留 `@custom-variant dark (&:is([data-theme$="-dark"] *));`——去掉它，上游原语里的全部 `dark:` 工具类会静默失效（实测产物中 `prefers-color-scheme` 出现 0 次）。
- **导入改写**：上游原语用 npm 的 `cn` 包与 `@/components/ui/*` 路径；本仓改为 `@/shared/lib/utils` 与相对导入 `./button` / `./separator`，并删掉 `"use client"` 指令（Vite SPA 下无意义）。
- **同名 token 不是自引用**：`--shadow-xs` 等顶层 token 名与 Tailwind 主题名同名。`@theme inline { --shadow-xs: var(--shadow-xs); }` 不会自引用——`inline` 不产出该自定义属性，工具类直接内联 `var(--shadow-xs)`，值由 `[data-theme]` 作用域提供（实测 `.shadow-xs{--tw-shadow:var(--shadow-xs)}`）。
- **状态变体必须重映射（最容易踩的坑）**：配方用的是布尔简写（`data-open:` / `data-checked:` / `data-horizontal:` …），Tailwind 把它们编译成 `[data-open]` 这类存在性选择器；但本仓装的 `radix-ui` 只发 `data-state` / `data-orientation`。**缺了 `style.css` 里那 8 条 `@custom-variant` 重映射，这些状态样式会静默失效**——实测表现为 Switch 轨道透明（控件不可见）、Dialog/Select 无进场动画、Separator 无尺寸。新增原语时若用到别的简写（如 `data-highlighted:`），必须同步加重映射。已知残留：`data-placeholder:` 无人设置（上游同样是死代码），只影响未选值时的占位文字颜色。
- **sonner 覆盖提权**：sonner 自带 `[data-sonner-toast][data-styled=true]` 规则且在本文件之后进入 bundle，覆盖必须加 `html` 前缀（`html [data-sonner-toast]…`）。

8. Toast 换 sonner（**可拆为独立提交 0b 以便单独回滚**）：删 store 队列与 `.toast` recipe，约 57 处 `store().toast(msg, type)` 改为直连 `toast.success/error/info`（`guard/ops.ts` 32 处、`ModelView` 16 处、`HomeView` 15 处最集中），`guard.test.tsx` 的 4 条 `getState().toasts` 断言改为 spy sonner。

**步骤 1..6 — 逐屏收敛**（每屏一次提交，可独立验证）

`home` → `guard`（DropdownMenu 接管 `.guard-param-actions`）→ `models`（Select 主战场；DropdownMenu 复用；Table 视密度重做而定）→ `integration` → `settings` → `updater`

每屏的完成标志：该屏不再引用已删除的类串、无裸颜色字面量、所有交互元素五态齐备、间距落在 R8 带内、`check-craft` 通过。

**保留项**：产品语义 recipe（`.select-card` / `.status-badge` / 时间轴 / 模式预览 / 状态指示器）不是原语，保留为 recipe 类，但必须改读 token 且不得写死颜色。

## 5. 一致性门（`scripts/check-craft.mjs`，v1）

1. `src/shared/lib/ui.ts` 不得存在。
2. `src/**/*.tsx` 与 `src/style.css` 不得出现颜色字面量（`#hex` / `rgb(` / `rgba(` / `hsl(` / `oklch(`）；每处例外必须同行带 `craft-allow-color` 标记注释。`src/themes.css`（生成物）不在扫描范围。
3. `src/style.css` 的 `@theme inline` 必须含 shadow 阶梯、`radius-2xl/3xl/4xl`、`font-heading`、`sidebar`、`chart` 桥。
4. 每个 `src/shared/components/ui/*.tsx` 必须含状态四元组片段；容器/展示型原语（Separator/Card/Dialog/Tooltip）与用 roving focus 的菜单类（DropdownMenu 用 `data-highlighted`/`:focus` 表达高亮）用 `craft-allow-no-states` 声明豁免并写明原因。
5. `src/**/*.tsx` 不得出现原生 `<select`。
6. `src/shared/store.ts` 不得再出现 toast 队列状态（`toasts`）；`src/style.css` 不得出现 `.toast` recipe。

## 6. 与既有决策的关系

- **不改主题模型**：[ADR 0008](adr/0008-tweakcn-token-theming.md) 的「主题族 × 模式 × `[data-theme]` token」与生成链完全不动；组件层只引用语义 token。
- **补完 ADR 0010 的遗留**：[ADR 0010](adr/0010-shell-frontend-react-rewrite.md) 声明了 React + shadcn/ui（`components.json` 指向 `@/shared/components/ui`），但该目录一直不存在——本规范与 [ADR 0011](adr/0011-shell-component-primitive-layer.md) 把它落地。
- **CSP 约束**：sonner 的样式经 Vite 作为普通 CSS 导入（无运行时注入），但其组件使用内联 `style` 属性，因此 `style-src` 必须保留 `'unsafe-inline'`——当前 `src-tauri/tauri.conf.json` 已是 `style-src 'self' 'unsafe-inline'`，`script-src` 仍不含 `unsafe-inline`，不受影响。将来收紧 style-src 前必须先处理 sonner。
- **与主题族正交**：不得为任何族写组件特例；R1/R2 的 ring 与 token 阶梯自动覆盖全部族。
