# 壳前端 React 重写实施计划

决策依据：[ADR 0010](adr/0010-shell-frontend-react-rewrite.md)。本计划是可执行的工作分解，随实施进度更新勾选。

## 目标与非目标

**目标**：壳前端（`src/` + `index.html` 静态视图）重写为 React 19，可维护性提升，**严格保行为**。

**非目标**：不改视觉/交互设计；不动 vendor/dashi-taskboard；不动 Rust 侧（IPC 命令面不变）；不追求全视图测试覆盖。

## 目标结构

```
src/
├── main.tsx                  # 入口：主题预渲染引导 + i18n 就绪 + 挂载 <App/>
├── App.tsx                   # 壳：activeView 切换 + 事件桥 + 初始化 + 3s 轮询
├── features/
│   ├── guard/                # 看守域：GuardView + GuardSettingsSection + 两个弹窗 + ops + 测试
│   ├── home/                 # 主页：进程状态卡、总状态指示器、Codex 重启确认
│   ├── settings/             # 设置页壳 + 通用/外观/网络/模式分区
│   ├── skill/                # Skill 安装
│   ├── integration/          # fastctx + dsh 卡片
│   └── updater/              # 关于分区（更新源健康/检查/进度）
└── shared/
    ├── commands.ts           # 集中式类型化 IPC（命令名全仓唯一）
    ├── events.ts             # Tauri 推送事件类型化订阅（事件名全仓唯一）
    ├── store.ts              # Zustand：导航/配置草稿/进程/看守/更新器/主题/toast
    ├── config.ts             # currentConfigDraft（旧 readConfigFromUI 语义）
    ├── types.ts              # IPC 载荷与视图类型
    ├── theme.ts              # data-theme 解析（纯函数）
    ├── theme-families.ts     # 生成物：41 族 manifest
    ├── i18n/                 # react-i18next 初始化 + en/zh-CN 字典
    ├── lib/                  # ui 类串、cn()、fmtTs
    ├── components/           # Toaster、Modal、SelectCard
    └── test/                 # vitest setup
```

依赖方向：`shared` ← `features` ← `App`/组合根；feature 之间禁止互相 import（ADR 0009 精神延续；SettingsView 作为组合根引用 guard/updater 特征的分区组件）。

**shadcn 实收说明**：三轮共识选定 Q6(b) 无头组件库；实施中所有应用内弹层（看守两弹窗、Codex 重启确认）复用既有 `.modal-overlay/.modal-card` 主题化样式即可逐像素保行为，引入 Radix/shadcn 反而会带入焦点圈禁、ESC 关闭等新行为，与 Q4 严格保行为冲突。故实收为零组件，`components.json` 与 `cn()` 保留作为后续入口。

## 单元序列

| # | 单元 | 完成验证 |
|---|------|----------|
| 0 | ~~**冒烟清单补全**~~ ✅ 已完成（本文档第 4 节） | 清单评审通过 |
| 1 | ~~工具链~~ ✅ | `pnpm dev` 起、`pnpm build` 过、主题生效 |
| 2 | ~~壳架~~ ✅ | 5 视图可切换；切语言/主题即时生效 |
| 3 | ~~主页 + service~~ ✅ | tsc/build 绿 |
| 4 | ~~设置页六分区~~ ✅（看守分区仅总开关，文件列表随单元 8 落地） | tsc/build 绿 |
| 5 | ~~Skill 安装视图~~ ✅ | tsc/build 绿 |
| 6 | ~~集成视图（fastctx + dsh）~~ ✅ | tsc/build 绿 |
| 7 | ~~updater~~ ✅（AboutSection 归入 updater 特征域） | tsc/build 绿 |
| 8 | ~~看守视图 + Vitest~~ ✅ 12 用例全绿；CI 加 `vitest run` | 测试绿 + tsc/build 绿 |
| 9 | ~~清扫~~ ✅ 旧 11 模块已删、纯模块归位 shared/、宽松声明已除；**全量冒烟待用户执行** | tsc/vitest/build 全绿 |

每单元一个提交；中间态不发布、不合并 main，全部通过后一次性合并。

**冒烟执行前提**：已安装的 Codex Pro Max 正在运行时，dev 实例会被单实例插件拦截退出（2026-08-15 实测拦截生效，顺带验证了单实例行为）。请先退出已安装实例，再 `pnpm tauri dev`，对照第 4 节逐条核对。

## 4. 冒烟清单（验收 spec，单元 0 产出）

格式：操作 → 预期。每条对应现状可感知行为（源码依据：shell/guard/service/core/fastctx/updater/dsh/nav + index.html）。

**2026-08-15 自动化冒烟结果：90/90 通过**（headless Chromium + Tauri IPC Mock harness：`smoke.html` + `src/smoke/`，驱动 90 项断言 + 16 张截图逐张目检 + 控制台零错误）。冒烟抓获一个 dev 环境致命 bug 并已修复：vite 预打包默认递归扫全仓 html，把 vendor/dashi-taskboard 的 react-dom/scheduler 捆入依赖 → 双 React 实例白屏（`optimizeDeps.entries` 限定入口修复；生产 build 单入口不受影响）。

**仍需真实应用复核的残留项**（Mock 无法覆盖，需退出已安装实例后 `pnpm tauri dev` 人工过一遍）：原生 ask/open 系统弹窗外观与按钮、托盘驻留与关窗最小化、窗口尺寸/位置记忆、真实文件写入与 dashi-backups 备份、真实更新下载与安装、系统通知弹出、OS 登录自启动、fastctx/dsh 真实 CLI 副作用。单实例拦截已在 dev 启动时实测生效 ✓。

### 启动序列
- [ ] 冷启动 → 首屏主题无闪切（主题在首个 await 前应用）
- [ ] 启动 → 界面语言 = Rust 解析结果；`get_resolved_language` 失败时回落英文；`<html lang>` 同步
- [ ] 启动 → 设置表单填充配置（taskboard/node/codex 路径、host/port/cdp、两个模式开关、托盘开关、看守总开关、语言卡片选中态）
- [ ] 启动 → 自启动开关反映 OS 注册项实时状态（读取失败视为关）
- [ ] 启动 → 静默请求一次系统通知权限（拒绝不打扰）
- [ ] codex 路径为空或失效 → 自动探测真实安装位置、回填输入框并落盘
- [ ] 启动 → 三项路径校验结果显示；关于页版本号显示（失败显示 "unknown"）
- [ ] 启动 → 更新源健康检查 + 静默检查更新（有新版本才提示）
- [ ] 启动后每 3 秒轮询进程状态与看守视图（看守视图隐藏时跳过；内容不变不重渲染；焦点在看守视图输入框内时当轮跳过，不抢焦点）

### 导航
- [ ] 顶部 5 按钮切换对应视图，当前按钮高亮；设置/集成按钮是 toggle（再点回主页）
- [ ] 进入 Skill → 刷新 Skill 状态；进入看守 → 强制刷新看守视图 + 文件列表；进入集成 → 刷新 fastctx + dsh 状态
- [ ] 设置侧栏 6 分区切换、当前项高亮；进入看守分区 → 刷新文件列表；保存 footer 仅在外观/看守/关于分区隐藏
- [ ] 看守总开关关闭 → 顶部「看守」Tab 隐藏；若当前正在看守页 → 自动跳回主页

### 主页
- [ ] 两个进程卡：状态徽标五态（Running/Stopped/Starting/Stopping/Failed）+ 消息行（空显示 "-"）
- [ ] 单卡按钮禁用逻辑：running/starting 禁 Start；仅 running 可 Stop；仅 running 可 Open（taskboard）
- [ ] Start All（无 taskboard 路径）→ 错误 toast 并跳转到设置-通用分区
- [ ] Start All → 按钮禁用并显示 "Starting..."；先落盘配置再启动；完成 toast + 刷新状态；按钮复原
- [ ] 启动遇 Codex 已运行无 CDP（Windows）→ 应用内确认弹窗（主题化、确认键聚焦）；确认 → 显示重启加载遮罩、退出 Codex 重试；取消 → "Launch cancelled" toast
- [ ] Stop All → 按钮禁用显示 "Stopping..."；完成 toast + 刷新
- [ ] 全局按钮联动：任一 running/starting → Start All 禁用；全部 stopped/failed → Stop All 禁用
- [ ] 总状态指示器聚合：任一 failed → "Service issue"（✕）；有过渡态 → "Services starting"/"Partially running"（▶）；全 running → "All services running"（✓）；全停 → "Services stopped"（✕）；图标随态切换
- [ ] `status-update` 事件 → 对应卡片实时更新（不等轮询）

### 设置-通用
- [ ] 语言三卡片选中态 = 当前设置；点击切换 → 落盘 + Rust 重建托盘 + 界面即时重渲染（静态 + 全部动态文本）；失败错误 toast
- [ ] Browse（taskboard）→ 目录选择器；Browse（node）→ 文件选择器；Browse（codex）→ macOS 选 .app 目录 / Windows 选 .exe 文件
- [ ] Use Bundled → 填充内置 taskboard 路径并触发校验 + 成功 toast；未找到 → 错误 toast
- [ ] 任一配置输入框 input 事件 → 即时重新校验三项路径
- [ ] 路径校验：taskboard 空则不显示；Valid/Invalid/Check failed；node 空路径也检测（系统 PATH）显示版本号或 Unavailable；codex Exists/Not found
- [ ] 托盘开关、模式开关改动不即时落盘（等 Save）；自启动开关即时写 OS 注册项，失败回退勾选态 + toast
- [ ] 打开日志目录 → 系统文件管理器打开（失败 toast）

### 设置-外观
- [ ] 模式三卡（跟随系统/亮/暗）选中态 + aria-pressed；点击即生效并写 localStorage
- [ ] 41 族色板卡网格：每卡局部 `data-theme` 渲染该族亮主题缩略（4 色点 + 2 色条）；当前族有选中勾
- [ ] 切族 → 全界面即时换肤并写 localStorage；localStorage 中非法族 id 静默回落 vercel
- [ ] 跟随系统模式下切换 OS 亮暗 → 界面即时跟随

### 设置-网络 / 模式
- [ ] host/port/cdp 编辑；保存时空值回落默认 127.0.0.1 / 47823 / 9231
- [ ] 模式开关文案随状态：开 = "Separate window mode (does not restart Codex)"；关 = "Full launch mode (restarts Codex)"
- [ ] 自动打开浏览器开关文案随状态：开 = "Open browser automatically on start"；关 = "Do not open browser automatically"

### 保存
- [ ] Save Settings → `update_settings`（不回滚看守状态）→ 重新 load_config 同步 guardState → 成功 toast；失败错误 toast

### 设置-看守（文件管理）
- [ ] 文件列表为空 → "No files yet"
- [ ] 内置文件：删除按钮禁用显示 "Built-in"，有 Detect 按钮；自定义文件：有 Delete、无 Detect
- [ ] 检测记录三态文案：path matches / actually at \<path\> / file not found（均带时间戳）；无记录不显示
- [ ] 首次进入（内置文件无检测记录）→ 自动检测一次并落盘，之后只读记录
- [ ] Detect 检测到路径不一致 → 原生 ask 确认（warning）→ 确认则更新配置路径 + toast；一致/未找到（手动检测）→ 对应 toast
- [ ] + Add File → 弹窗（标题 "Add Guard File"、提交键 "Add"）；名称/路径必填校验；Pick… → 原生文件选择器 → 自动转相对路径、带入文件名与格式（toml/json/md）
- [ ] Edit → 弹窗填充现值、格式下拉禁用、标题 "Edit Guard File"、提交键 "Save"；保存只更新 name/file
- [ ] Delete → 原生 ask（文案含「已写入值不回滚」）→ 确认删除 + toast + 刷新列表与看守视图
- [ ] 弹窗点遮罩关闭；取消键关闭

### 看守视图
- [ ] 分组卡：组名、`~/.codex/<file>` 路径、组级错误（TOML 解析失败等）红色显示
- [ ] 参数卡：label + 「?」帮助悬浮（description + TOML path）；状态徽标四态（Match=绿/Drift=红/Missing=黄/Error=红）；Current 行显示实际值或错误（match 绿色/其他红色）
- [ ] 编辑器按值类型：bool → 开关 + "true/false (recommended X)" 文案；int → number 输入；string → 文本输入；text → 多行 textarea；无值类型 → "No editable value..." 提示
- [ ] 锁定参数：编辑器与启用开关均 disabled；显示 "Last checked | Last auto-restored" 时间行；锁定按钮变 Unlock
- [ ] 未启用参数的 Lock 按钮禁用（须先启用才能锁）
- [ ] bool 开关点击 → 值取反落盘 + 强制刷新；int 输入非整数 → 错误 toast + 强制刷新
- [ ] 启用开关：关→开 = apply（写入文件，成功 "Applied" toast）；开→关 = disable（"Disabled" toast，不回滚）
- [ ] Lock/Unlock → 对应 toast + 刷新
- [ ] 自定义参数 Delete → 原生 ask（含「备份在 dashi-backups」说明）→ 删除 + 刷新
- [ ] 组内「+ Add Parameter」→ 打开添加弹窗并预选该组文件；底部「+ Add Custom Parameter」→ 打开弹窗
- [ ] 添加弹窗联动：toml_key/toml_absent 显示 TOML Path 与值类型行；file_overwrite/markdown_block 隐藏之并固定 text 类型；值类型 none 隐藏默认值行；text 类型时默认值输入框换为 textarea（反之换回，内容保留）
- [ ] 添加校验：ID/名称/目标文件必填；toml 模式 TOML Path 必填；int 默认值非整数报错
- [ ] 添加成功 → 清空表单、关闭弹窗、刷新视图；toast "Custom parameter added"
- [ ] Open schema file → 系统打开 schema 文件；失败回落复制路径到剪贴板 + toast
- [ ] 轮询刷新不重建 DOM 当内容未变；正在输入时不刷新

### Skill
- [ ] 状态徽标三态：Installed（绿）/ Installation mismatch（黄）/ Not installed（灰）+ detail 文本；检测失败 → 红色 + 错误详情
- [ ] Install（无 taskboard 路径）→ 错误 toast；成功 → 结果显示在结果区 + 成功 toast；失败 → 结果区显示失败原因 + 错误 toast；完成后刷新状态

### 集成-fastctx
- [ ] 状态文案四态：Working… / Not installed / Integrated · \<version\> / Installed (\<version\>), not integrated
- [ ] 未安装 → 显示安装引导文案；有新版 → 右侧显示版本胶囊
- [ ] 开启开关（未安装）→ 先 npm 全局安装（toast 提示）再接入；接入成功 toast + 需重启 Codex 提示；自检 FAIL → 额外错误 toast（含失败行）
- [ ] 关闭开关（已接入）→ 原生 ask 确认摘除（warning，含数据删除说明）；取消则开关回弹
- [ ] busy 期间再点开关 → 无效且开关回弹
- [ ] Open fastctx Console（未安装）→ 错误 toast；已安装 → 打开控制台

### 集成-dsh
- [ ] 状态文案按优先级链：Node 未检测到 → dsh 未安装 → web 未运行 → Tailscale 未就绪 → MagicDNS 未启用 → 代理未运行 → serve 未配置 → Remote access ready
- [ ] 版本检测失败 → 红色错误行显示；已安装 → 版本胶囊；有 url → url 胶囊
- [ ] 按钮禁用：start 仅 busy 禁用；stop 需 dsh 或 proxy 在跑；open 需有 url
- [ ] 更新按钮用 hidden 属性控制（非类），仅有新版显示 "Update to vX"
- [ ] 一键远程访问 → 时间轴 8 步全 pending 起跑，dsh-step 事件逐步推进；步骤标记五态（✓/✕/转圈/–/○）；失败步骤节点内嵌问题 + 解决方案
- [ ] 安装成功 → toast + 时间轴回到检测驱动视图；失败 → 保留事件时间轴（问题持续可见）
- [ ] Stop → toast + 回检测驱动时间轴；Update → 成功 toast 显示新版本 + 回检测驱动
- [ ] 未跑过一键安装时 → 时间轴由实时检测推导（已满足步 done，其余 pending）
- [ ] 开机自启开关 → 即时生效 + toast；失败回退勾选态

### 关于 / 更新
- [ ] 更新源健康：就绪 → "Ready" 绿色 + 隐藏帮助行；未就绪 → 显示原因 + 帮助行（Setup Guide / Config Template 链接浏览器打开）
- [ ] Check for Updates → 按钮禁用显示 "Checking..."；有新版 → 显示更新行（版本 + release notes，空 notes 隐藏）+ 按钮变 "Update Now" + toast；已最新 → toast；失败（非静默）→ toast
- [ ] Update Now → 按钮禁用 "Updating..." → `install_update`（带期望版本）→ 成功 toast + 隐藏更新行；失败 toast + 按钮回 "Update Now"
- [ ] 下载进度事件四阶段文案：Downloading vX: N%（无百分比时显示 MB）/ 重试 (n/m) / Installing… / Installation complete, restarting…；结束后进度条归零隐藏
- [ ] GitHub 链接 → 浏览器打开

### 横切
- [ ] toast：三类型配色、3 秒后 0.3s 淡出移除、堆叠在右下
- [ ] 时间戳格式：按界面语言 locale、24 小时制；null 显示 "—"
- [ ] 全部动态文本双语；静态文本含 placeholder/title/aria 翻译；看守参数 label/description 按界面语言取 `_en` 资源
- [ ] 壳（Rust 侧，端到端核对）：单实例二次启动激活已有窗口；关窗按设置最小化托盘；窗口尺寸/位置记忆恢复；壳通知仅进程事故


## 风险

- **最大风险单元是看守视图**（607 行、四种写入语义）——故压轴做且唯一配测试。
- **事件桥时序**：Tauri 事件在 React 挂载前到达会丢——store 初始化先于 `listen` 注册，启动序列在单元 2 验证。
- **innerHTML → JSX 的转义行为差异**：现状统一 `escapeHtml`，JSX 默认转义；注意 `SERVICE_INDICATOR_SYMBOLS` 等内嵌 SVG 需转为组件而非字符串注入。

## 明确不做

- 不引入路由库、TanStack Query、成品组件库、CSS-in-JS
- 不改 themes.css / build-themes.mjs / theme-families.ts 生成链
- 不为 vendor 与壳共享任何前端代码
