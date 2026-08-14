# 壳前端迁移 React（推翻 ADR 0009 的 vanilla TS 决策）

壳前端从 vanilla TS + ES modules 重写为 React 19，一次性重写（非渐进共存），严格保行为。[ADR 0009](0009-vanilla-ts-es-modules-frontend.md) 当时拒绝 React 的前提是「壳 UI 交互面窄（表单、列表、开关、弹窗）」，并约定「交互面显著增长时重审」。该触发条件现已成立：看守域（四 apply_mode × 锁定状态机 × 文件管理）、dsh 时间轴与 updater 进度等事件推送视图落地后，命令式 DOM 的维护成本集中爆发——典型如切语言需在 `rerenderDynamicText` 手动重跑十余个渲染函数、动态内容靠 `delegate` 委托分发与渲染函数隐式对齐。重写动机是可维护性，不是生态或视觉重做：行为、主题、i18n 文案、原生弹窗逐条冻结。

技术栈：React 19（对齐 vendor/dashi-taskboard，全仓统一）+ shadcn/ui（复制源码、按需引入；其 token 约定与 tweakcn 41 族主题同源，见 [ADR 0008](0008-tweakcn-token-theming.md)，零适配）+ Zustand（Tauri 事件桥直写 store）+ react-i18next（字典原样复用）+ 集中式类型化 commands 模块（命令名全仓唯一）。不引路由库（桌面应用无深链，`activeView` 状态切换）。Vitest + Testing Library 只覆盖看守域，CI 加 `vitest run` 门。

**Considered Options**：

- **Strangler 渐进共存** —— 拒绝。壳前端仅约 2200 行，两套事件体系与两个状态源并存的桥接成本超过重写本身；大爆炸在分支内仍按单元推进、逐单元验证，只是不发布中间态。
- **保持 vanilla TS** —— 拒绝。状态联动与事件推送的增长使「显式调 render」模式成为 bug 温床（漏调即界面过期），这正是 ADR 0009 预设的重审条件。
- **成品组件库（MUI / AntD 等）** —— 拒绝。自带主题系统与 41 族 `[data-theme]` token 机制直接冲突，等于推翻 ADR 0008。
- **Vue / Svelte / Solid** —— 拒绝。vendor/dashi-taskboard 已是 React 19，引入第二个框架无理由。

**Consequences**：

- 本 ADR 取代 [ADR 0009](0009-vanilla-ts-es-modules-frontend.md)；其依赖方向精神保留并改写为：`shared` ← `features` ← 壳（`App`/store 接线），**禁止 feature 之间互相 import**。
- 原生 `ask()`/`open()` 等 Tauri 插件弹窗保留；shadcn 组件只用于应用内弹层（文件管理弹窗、下拉等）。
- 前端首次引入测试基建（Vitest），但范围限定看守域，不追求全视图覆盖。
- i18n 挂载机制从 `data-i18n` 扫描改为 `useTranslation` 订阅；语言字典、解析规则、落盘行为不变。
- vendor/dashi-taskboard 不受影响；Rust 侧零改动（IPC 命令面不变）。
