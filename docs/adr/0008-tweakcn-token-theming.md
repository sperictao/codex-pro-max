# 移除 daisyUI，切换 tweakcn（shadcn token）主题体系

壳 UI 的主题底座从 daisyUI 换为 tweakcn 预设（shadcn 语义 token）：42 个预设全量本地化——`scripts/build-themes.mjs` 从 tweakcn registry 拉取，生成 `src/themes.css`（84 个 `[data-theme]` scoped 块 + `@font-face`）、`src/theme-families.ts` manifest，并把预设引用的 Google 字体下载进 `assets/fonts/`。daisyUI 组件类（btn/input/select/toggle/badge/link 等）的全部调用点展开为 shadcn 语义的 Tailwind utility 串。「族 × 模式」模型与 `data-theme` 机制不变，配对表坍缩为 `<族id>-light|dark` 命名约定；默认族 vercel，取代 geist 成为视觉基准。

**Considered Options**：

- **tweakcn 只当调色板**（色值映射成 daisyUI 主题，组件不动）—— 拒绝。daisyUI 与 shadcn 的 token 语义划分不同（base 三层 vs background/card/muted 面），每次跟随上游都要手工对译，且组件审美仍是 daisyUI 的。
- **桥接层**（`--color-base-100: var(--background)` 等，保留 daisyUI 组件）—— 拒绝。多一层间接只为留住一个不再投资的依赖，debug 时变量链变长，收益为零。
- **换 vanilla 组件库**（Preline / Flowbite）—— 拒绝。等于再押注一个新的组件库审美，而壳 UI 的组件面很窄（按钮/输入/开关/徽章），utility 直写已够用。
- **保留 Geist 为品牌字体**（丢弃预设字体）—— 拒绝。字体是预设外观的组成部分；42 族各自成立的前提是连字体一起随族。代价是 28 种 woff2 本地化（latin/latin-ext），完全离线、CSP `font-src 'self'` 不变。

**Consequences**：

- token 的上游事实来源是 tweakcn registry；`src/themes.css` / `src/theme-families.ts` / `assets/fonts/` 是生成物，不手改。重跑 `node scripts/build-themes.mjs` 主动跟随上游；`PRESET_IDS` 写死，上游新增预设不自动进入。
- 状态语义色不随族：`--status-ok` / `--status-warn` 为亮暗皆可的固定值（42 个预设无法各自重新定义「绿=运行中」），错误色走 `var(--destructive)` 随族。
- 自有复合类（`status-badge` / `select-card` / `toast` / `modal-card` 等）保留为 recipe 类，改读 shadcn token；vanilla 栈里 CSS 类即 cva recipe 的等价物。
- 按钮/输入/开关等原子组件不再有任何类名封装，utility 串直接写在调用点；改样式 = 全文替换对应串。
- 字体随族且全部本地打包，安装包增大约 4MB；预设字体均无 CJK，中文回落系统字体。
- 取代 [ADR 0007](0007-tailwind-daisyui-geist-theming.md)。
