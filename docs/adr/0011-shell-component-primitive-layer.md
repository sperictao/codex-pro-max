# 壳 UI 建立自有组件原语层（移植 shadcn radix-nova 配方）

壳 UI 的按钮、输入、下拉、开关、弹窗此前是 `src/shared/lib/ui.ts` 里的一袋类串常量加 `style.css` 的 recipe 类：85 个调用点各自拼串，同一批控件在焦点、禁用、错误、密度上互不一致（`focus-visible:` 全仓仅 4 处，`tracking-` 0 处，`tabular-nums` 1 处）。决定改为**复制 shadcn（`radix-nova` 风格）原语源码进 `src/shared/components/ui/`、删除 `ui.ts`、把调用点收敛到原语**；浮层身份改用取自 `--foreground` 的 1px 描边（`ring-1 ring-foreground/10`），阴影只按语义阶梯使用生成物里的 `--shadow-*` token；新增依赖 `radix-ui`（统一包）、`class-variance-authority`、`lucide-react`、`tw-animate-css`、`sonner`。配方来自 `arhamkhnz/next-shadcn-admin-dashboard` @ `3858591`（MIT）的一次性移植，成文规约见 [craft-spec.md](../craft-spec.md)。

**Considered Options**：

- **保留 `ui.ts` 类串层，只在其内部调字串** —— 拒绝。字符串常量层没有变体、没有组合、没有状态契约，85 个调用点就是 85 个漂移点；实测该层已出现 `text-sm` 与 `text-xs` 并存的死字串、`ring-2` 的旧焦点写法、`rounded-md` 硬编码，且 `INPUT` 的 `shadow-xs` 落在未桥接的 token 上（实际用的是 Tailwind 默认阴影，与主题族无关）。"更精致"在此结构下无从判定。
- **只借用视觉结果，交互组件自写（不引 radix）** —— 拒绝。该配方的状态选择器直接挂在 Radix 的 data 契约上（`data-checked` / `data-open` / `data-placeholder` 与 `--radix-*-content-transform-origin`），自写等于逐个重实现契约；焦点陷阱与键盘导航自研是 a11y 最易翻车处，而 ADR 0010 已预先批准这条路（`components.json` 早已声明 `ui` alias）。
- **引入成品组件库（MUI / AntD）** —— 已在 [ADR 0010](0010-shell-frontend-react-rewrite.md) 拒绝，理由不变：自带主题体系与 `[data-theme]` token 机制冲突。
- **逐屏打磨、不留规约** —— 拒绝。无规约即无法判定"是否更精致"，且必然再长出 `.header-btn` 那样的 bespoke 类。
- **一次全引上游 60+ 原语** —— 拒绝。零调用点的原语属于"以后可能用得上"的保留路径，违反 Clean；改为按调用点引入，同一次提交里接线。
- **保留 store 的 toast 队列、让 sonner 只当渲染器** —— 拒绝。sonner 自带队列与定时器，store 再持一份就是两个定时器、两份状态（我们 3.3s 淡出、sonner 默认 4s），属于第二份真相；且测试断言所依赖的 `toasts` 数组会退化为只为测试存在的死状态。

**Consequences**：

- `src/shared/components/ui/` 成为壳内组件样式的单点；`components.json` 的 `ui` alias 与 `iconLibrary` 从此名副其实，`style` 由 `new-york` 修正为 `radix-nova`。
- `src/shared/lib/ui.ts` 删除，12 个视图 85 个调用点改走原语——本次最大的机械改动，按屏推进、每屏独立可验证（顺序见 craft-spec §4）。
- 层级与主题族正交：ring 取自 `--foreground`，真阴影走生成物 token；因此 `@theme inline` **必须先补齐** `shadow-2xs…2xl`、`radius-2xl/3xl/4xl`、`font-heading`、`sidebar-*`、`chart-*` 桥，否则移植的原语会静默回落到 Tailwind 默认值、不再跟随主题族。
- 新增 5 个依赖（`radix-ui` 为统一包）。**不引** cmdk（命令面板属独立需求）、**不引** Checkbox（0 真实调用点）。
- 规约由 `scripts/check-craft.mjs` 强制并纳入 `pnpm test`：文档纪律从"靠 review"变为可执行门，防止未来新屏回退。
- Toast 改用 sonner：`Toaster.tsx` 变成薄包装，store 的 toast 队列状态（`toasts` / `dismissToast` / `ToastItem` / `ToastType`）与 `style.css` 的 `.toast` recipe 一并删除，约 57 处调用点直连 `toast.success/error/info`，`guard.test.tsx` 的 4 条队列断言改为 spy sonner。sonner 的样式经 Vite 作为普通 CSS 导入，但其组件使用内联 `style` 属性，故 `style-src` 必须保留 `'unsafe-inline'`（当前 CSP 已满足）。
- 产品语义 recipe（`.select-card` / `.status-badge` / 时间轴 / 状态指示器）不是原语，保留为 recipe 类且只读 token。
