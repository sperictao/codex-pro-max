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
- **schema** — 描述托管参数集合的 JSON 文件。落盘于 launcher 配置目录，启动时与内置 schema 合并（同 id 内置覆盖磁盘，磁盘独有条目保留），UI 完全由合并结果驱动。
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
