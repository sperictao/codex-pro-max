# CONTEXT

## Codex 配置看守（Codex Config Guard）

启动器的功能域：对 `~/.codex/` 下的配置类文件做**基于 schema 的参数托管与锁定**。

### 术语

- **托管参数（Managed Parameter）** — schema 中声明的一条可管参数。含：分组、目标文件、推荐值、用户可改的值、启用状态、锁定状态。
- **分组（Group）** — 按目标文件划分，如 config.toml、auth.json、AGENTS.md、agents/default.toml。
- **自定义参数（Custom Parameter）** — 用户在 UI 增删的托管参数，id 以 `custom.` 开头，落盘于磁盘 schema；删除只停看守，不回滚已写入的值。
- **看守文件（Guard File）** — 看守目标文件列表（内置 config.toml / AGENTS.md / agents/default.toml + 自定义），持久化在 `LauncherConfig.codex_guard.files`；视图分组与轮询只覆盖列表内文件，路径不可重复。
- **检测记录（Detection）** — 对文件实际路径的检测结果（命中路径或“未找到”+ 时间戳），随 GuardFile 落盘；首次进入设置页自动检测一次，之后直接读记录，手动“检测”按钮才重扫；检测到路径与配置不一致时弹窗确认后才改配置。
- **启用（Apply）** — 把参数的（修改后或默认）值写入 codex 对应文件。
- **锁定（Lock）** — 已启用参数的看守状态；锁定期间轮询发现实际值与配置值不一致时自动改回。
- **轮询（Poll）** — 周期性比对锁定参数的实际状态与配置状态。
- **schema** — 描述托管参数集合的 JSON 文件。落盘于 launcher 配置目录，启动时与内置 schema 合并（同 id 磁盘覆盖内置——用户定制内置参数；但 `label_en` / `description_en` / `default_en` 英文资源始终以内置为准，磁盘独有条目保留），UI 完全由合并结果驱动。
- **漂移（Drift）** — 锁定参数的实际状态与配置值不一致。轮询发现漂移即自动改回（写入前备份），配置页记录上次校验/恢复时间，不弹通知。
- **备份（Backup）** — 任何写入前把目标文件当前内容复制到 `~/.codex/dashi-backups/<文件名>.<时间戳>.bak`，每文件保留 20 份，无 UI 还原入口。
- **路径（Path）** — TOML 参数在文件中的点分位置，如 `features.image_generation`、`features.multi_agent_v2.enabled`、`agents`（toml_absent 的目标）。写入/比对/删除都按路径在解析后的 TOML 树上定位，中间表不存在时写入会逐级创建。
- **apply_mode** — 参数写入/校验的方式，四选一：
  - `toml_key` — 写入/更新 TOML 某键；比对值。
  - `toml_absent` — 确保某 TOML section 不存在；它再出现就再删（用于 multi_agent_v1 的 `[agents]` 块）。
  - `file_overwrite` — 整文件内容即值；比对全文哈希。
  - `markdown_block` — 用 `<!-- dashi:begin/end 名称 -->` 标记圈定的 Markdown 托管区块；比对区块内容。

### 语义边界

- 锁定即只读：改值须先解锁 → 改 → 启用 → 再锁。
- 不回滚：解锁或全局关闭只停止看守，已写入的值留在原地。
- 看守生命周期：仅在 launcher 运行期间轮询（固定 60s），launcher 关闭即停。
- 全局开启本身只检查不写入；写入只发生在手动启用或锁定后漂移恢复。
- 文件移出看守列表即不再生效：该文件的参数从视图消失，轮询同样跳过（列表是看守范围的唯一事实来源）。
- TOML 解析失败：跳过该文件全部校验并在分组显示错误，绝不重写整文件；文件缺失时启用则新建。

## Taskboard 集成

启动器的另一功能域：把 dashi-taskboard 应用打包进安装包并注入 Codex 桌面端。

### 术语

- **主仓库（Upstream）** — `chuspeeism/dashi-taskboard`，taskboard 代码的权威来源，变更以 PR 汇入。
- **Fork** — `sperictao/dashi-taskboard`，主仓库的 fork，向主仓库提 PR 的中转地。
- **Launcher 仓库** — `sperictao/dashi-taskboard-launcher`（本仓库），Tauri 壳。
- **Vendor 快照（Vendor Snapshot）** — taskboard 在 launcher 内 `vendor/dashi-taskboard/` 的 git submodule，指向 Fork、pin 具体 commit；升级由 launcher 显式 bump 指针。取代 v0.2.5 前的纯文件拷贝。
- **Bundle** — Tauri 构建时经 `tauri.conf.json` resources 映射打进安装包的 taskboard 文件集。

## 界面多语言（i18n）

启动器的横切关注：壳 UI 与 Rust 侧所有用户可见字符串的多语言。

### 术语

- **界面语言（Display Language）** — 启动器自身用户可见文本的语言。设置项三选一：跟随系统 / English / 中文。
- **跟随系统（Follow System）** — 默认取值。启动时按 OS 语言一次性解析：系统为中文则中文，其余一律英文。
- **默认语言（Default Language）** — 英文。跟随系统解析不到中文时的兜底，也是缺失翻译的兜底。
- **解析语言（Resolved Locale）** — 「跟随系统」经解析后得到的具体语言（en 或 zh-CN），启动时确定，改设置后立即重解析。

### 语义边界

- 覆盖启动器壳与 Rust 侧字符串（托盘、进程状态、错误消息）；vendor/dashi-taskboard 是独立应用，其 UI 语言不在此域。
- 切换即时生效：界面重渲染、托盘重建；已产生/已显示的消息不回溯重翻。
- 看守域的**托管参数** label/description 同样双语：schema 条目的 `label_en` / `description_en` 是英文资源，空则落回原文（自定义参数永远只有用户原文）。部分参数的 **default 值**（写入 codex 的内容，如 AGENTS.md 段落、default.toml 指令）也带 `default_en`：英文界面下期望值取 `default_en`，即**写入内容随界面语言**。语义后果：用户改过的 `state.value` 永远优先；未锁参数切语言会显示漂移（由用户决定重应用）；已锁参数切语言后 60 秒轮询内被自动恢复为新语言内容。无 `default_en` 的参数 default 值不随语言变化。
- 界面语言与文档语言无关：README、release notes 的双语是另一回事，不受此设置影响。

## FastCtx 集成

启动器的第三功能域：设置页「集成」区的开关，把 fastctx（`yc-duan/fastctx`，MCP 工具运行时）接入/摘除 Codex。

### 术语

- **接入（Integrate）** — 调 fastctx CLI `fastctx apply --yes` 完成注册：fastctx 自己写 `~/.codex/config.toml`（`[mcp_servers.fastctx]`、`features.code_mode.direct_only_tool_namespaces`、共享键 `tool_output_token_limit`）并固化二进制到 `~/.fastctx/bin/`；默认 Standard 输出档。与看守域的"启用"区分：接入对象是外部工具整体，不是单条托管参数。
- **摘除（Unapply）** — 调 `fastctx unapply --yes`：杀受管进程、移除 fastctx 配置、删除 `~/.fastctx` 受管数据；npm 全局包保留，可重新接入。
- **安装检测（Install Detection）** — 检测 fastctx 可执行文件是否在 PATH；每次进入「集成」区实时执行，不落盘、无记录。与看守域的"检测记录"同名不同义：看守检测持久化到配置且只比对文件路径，安装检测即时丢弃且测的是可执行性。
- **接入状态（Integration State）** — 以 `~/.codex/config.toml` 是否含 `[mcp_servers.fastctx]` 为唯一事实来源，启动器不存开关布尔值。
- **自检（Self-check）** — 接入成功后跑一次 `fastctx status`；FAIL 只 toast 警告，不阻塞、不回滚。

### 语义边界

- 启动器不写任何 fastctx 拥有的 TOML 键（`mcp_servers.fastctx`、`tool_output_token_limit`、`features.code_mode.*`），一律委托 CLI；这些键也不应加入看守，用户自行锁入造成的互相改回不管。
- 接受 apply 对 ChatGPT 桌面端配置的连带写入（fastctx 官方行为，两个 host 同时配置）。
- 接入/摘除后需重启 Codex 会话才生效，启动器只提示，不自动重启。
- fastctx 的安装、升级、输出档位、jobs 管理归 fastctx 自己的 TUI/CLI；启动器只做未安装引导（`npm i -g fastctx`）与「打开控制台」入口。

## 启动器壳（Shell）

启动器的横切关注：Tauri 桌面壳的进程模型、生命周期与系统交互边界。

### 术语

- **单实例（Single Instance）** — 应用同时只允许一个实例运行；第二次启动不新建进程，转为激活已有实例并显示主窗口（若最小化在托盘则恢复）。
- **自启动（Autostart）** — 开机登录时静默启动到托盘，不显示主窗口。设置项默认关，与 `minimize_to_tray_on_close` 相互独立，不复用。
- **窗口状态记忆（Window State）** — 记忆主窗口尺寸/位置/最大化，启动时恢复；恢复位置落在所有显示器可视区之外时放弃恢复、改为居中。
- **壳通知（Shell Notification）** — 系统级通知，触发点仅限受管子进程的生命周期事故（意外退出、启动失败）。
- **壳日志（Shell Log）** — Rust 侧日志落盘于 app log dir，release 级别 Info，单文件上限 2MB，超限后轮转（旧文件直接删除，仅保留当前一份）；设置页提供「打开日志目录」入口。

### 语义边界

- 多开即事故：看守轮询会对同一批 codex 文件双写、taskboard 子进程重复 spawn 抢端口，故以单实例强制禁止，不靠用户自觉。
- 壳通知不覆盖看守域：漂移恢复保持静默（沿用看守域「不弹通知」边界）；updater 流程不弹（用户手动发起，UI 已覆盖进度与结果）。
- 日志只进文件不进 UI：消费方式是「出事再翻」，无应用内日志查看器，不告警、不上报。

## 界面主题（Theming）

启动器的横切关注：壳 UI 的色彩主题系统（tweakcn token 化重构后，daisyUI 已移除）。

### 术语

- **主题族（Theme Family）** — 一个 tweakcn 预设的本地化实例，亮暗原生成对。全量 41 族，由 `src/theme-families.ts`（构建脚本生成的 manifest）枚举，选择器中的可选项即 manifest 内容。
- **模式（Theme Mode）** — 三选一：跟随系统 / 亮 / 暗。与主题族正交。
- **解析主题（Resolved Theme）** — 族 × 模式解析出的主题名，命名约定 `<族id>-light|dark`，赋给 `<html data-theme>`，对应 `src/themes.css` 中同名 scoped token 块；跟随系统时随 OS 切换重解析。
- **默认族（Default Family）** — `vercel`，视觉基准。localStorage 中的族值不在 manifest 时静默回落默认族。
- **色板卡（Swatch Card）** — 主题族选择器项：卡面自身带 `data-theme` 局部生效，直接渲染该族亮主题的色板缩略。
- **主题构建脚本（Theme Build）** — `scripts/build-themes.mjs`：按写死的 41 个预设 id 从 tweakcn registry 拉取 token 与字体引用，生成 `src/themes.css`（scoped 块 + `@font-face`）与 `src/theme-families.ts`；生成物提交进 git、不手改，重跑脚本即主动跟随上游（上游新增预设不自动进入，id 列表是唯一事实来源）。
- **字体本地化（Local Fonts）** — 预设引用的 Google 字体 woff2 全部由构建脚本下载进 `assets/fonts/`，CSP 保持 `font-src 'self'`，主题字体完全离线可用。

### 语义边界

- 主题选择器只列亮族：用户不直接选暗色主题，暗面永远由命名约定的配对决定（「每主题适配亮暗」的唯一语义）。
- 主题持久化在 localStorage（`theme` 模式、`theme-family` 族），不进 LauncherConfig、不同步 Rust；语言在 LauncherConfig，两者互不影响。
- 视觉基准以默认族 vercel 为准：其余 40 族是增值选项，组件样式不得为任何族写特例。
- 字体随族：各族的 font token 是该族外观的组成部分，产品不承诺单一品牌字体。
