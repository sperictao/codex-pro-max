# Tailwind/daisyUI 全量重构与 geist 自定义主题族

壳 UI 从 1307 行手滚 CSS 全量重写为 Tailwind v4 utilities + daisyUI v5 组件。主题系统采用「主题族 × 模式」二维模型：选择器只列亮族（geist 默认 + 内置亮色主题），暗面由配对表自动给出，未配对落内置 `dark`。

**Considered Options**：

- 桥接式（保留旧 CSS，只换变量层）与增量迁移 —— 拒绝。手滚 CSS 与 50 个 inline handler 是同一笔债，全量重写是唯一能把债清零的方案；重写期间 inline handler 改 addEventListener 的边际成本为零，CSP 的 `script-src` 得以摘掉 `unsafe-inline`。
- 主题平铺（35 个内置主题全列）—— 拒绝。内置主题全是单模式，平铺无法实现「每主题适配亮暗」，且选暗色主题会让模式卡语义失效。

**Consequences**：

- geist 族（`geist-light`/`geist-dark`）的 token 以 `DESIGN.md`/`DESIGN.DARK.md` 为唯一来源，是视觉回归基准；组件样式禁止为非 geist 族写特例。
- 新增主题族 = 配对表加一行 + 选择器自动出现，无需动组件。
- Geist Sans/Mono variable 字体自托管打包（OFL），界面字体不再依赖用户系统字体栈。
