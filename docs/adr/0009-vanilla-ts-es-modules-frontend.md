# 前端保持 vanilla TS + ES modules，不引入框架

`src/main.ts`（约 1800 行）拆分时定下的栈决策：前端保持 vanilla TypeScript + 原生 ES modules，按域切分为 core / service / guard / fastctx / updater / shell 六个模块 + `state.ts` 共享状态，不引入 React / Vue / Svelte 等任何框架。壳 UI 的交互面很窄（表单、列表、开关、弹窗），直写 DOM 已够用；「单文件太大」用 import 就能解决，不值得为此引入运行时与重写成本。构建链保持 vite + tsc 极简形态，严格 CSP（`script-src 'self'`）不变。

**Considered Options**：

- **引入 React / Preact 组件化** —— 拒绝。组件生态的收益在交互密集的富应用，壳 UI 的组件面窄且已被 Tailwind utility + recipe 类覆盖；引入框架 = JSX 构建链 + 运行时 + 全量重写，成本与问题规模完全不成比例。
- **Web Components** —— 拒绝。Shadow DOM 的样式隔离与 tweakcn token 的全局 `[data-theme]` 机制直接冲突（见 [ADR 0008](0008-tweakcn-token-theming.md)），且无跨组件异步数据流需求。
- **保持单文件** —— 拒绝。约 1800 行 / 22 个 section 已超单文件可维护阈值，guard 域独占约三分之一。

**Consequences**：

- 依赖方向单向：域模块 → `state.ts` / 工具；shell → 域模块；**禁止域模块之间互相 import**，跨域通信由 shell 的事件接线集中完成。
- 无响应式抽象：DOM 操作直写，状态变更后显式调 render；`i18n/`、`theme.ts`、`theme-families.ts` 是已成立的模块先例。
- 无框架也意味着无组件测试基建；前端质量门维持 tsc + 手动冒烟，不引入测试框架。
- 若将来交互面显著增长（复杂状态联动、嵌套组件树），本决策需重审。
