# Dashi Taskboard Launcher 设计文档

## 1. 概述

Tauri v2 桌面启动器，用图形界面替代手写命令，管理两个后台进程：

1. **Taskboard 服务** — dashi-taskboard 的 Node 服务（`npm start`）
2. **Codex 注入器** — 以独立 CDP 端口启动 Codex 并注入 Taskboard 面板

另集成 Tauri Updater 做应用自更新。

## 2. 架构

```
┌─────────────────────────────────────────────┐
│ 前端 src/ (vanilla TS + ES modules)          │
│ Guard 工作台：后端 DTO、进度事件、i18n/ARIA   │
└──────────────────────┬──────────────────────┘
                       ▼ typed Tauri commands/events
┌─────────────────────────────────────────────┐
│ Rust composition root / AppState             │
│ ConfigStore + GuardCoordinator + poll         │
└──────────────┬──────────────────┬───────────┘
               ▼                  ▼
      ┌────────────────┐  ┌──────────────────────┐
      │ Guard planner   │  │ Transaction engine    │
      │ format/semantic │  │ journal/snapshot/    │
      │ ownership/role  │  │ atomic write/recover │
      └────────┬───────┘  └──────────┬───────────┘
               └──────────────┬──────┘
                              ▼
                Codex files + Launcher state
```

## 3. 模块

### 3.1 config.rs

`LauncherConfig` 字段与默认值：

| 字段 | 说明 |
| --- | --- |
| `taskboard_path` | dashi-taskboard 仓库路径（可用内置打包路径兜底） |
| `node_path` | Node 可执行文件路径（要求 ≥22.5） |
| `codex_app_path` | Codex/ChatGPT.app 路径（自动探测候选路径） |
| `taskboard_port` / `taskboard_host` | 服务监听地址，默认 47823 |
| `cdp_port` | Codex CDP 调试端口 |
| `auto_open` | 服务起来后自动打开看板 |
| `separate_window_mode` | 独立窗口模式（保留现有 Codex 窗口） |

配置持久化到用户目录下的配置文件（`load_config` / `save_config`），Windows 路径经 `strip_unc` 归一化。

### 3.2 process_manager.rs

`ProcessManager` 托管子进程生命周期，状态机：

```
stopped → starting → running ⇄ stopping → stopped
                  ↘ failed
```

- `start_taskboard` / `stop_taskboard` — 启动/停止 Node 服务，健康检查通过才算 running
- `start_injector` / `stop_injector` — 启动/停止注入器（需常驻，Ctrl-C 等效于 stop）

### 3.3 main.rs

`#[tauri::command]` 暴露给前端：配置读写（`update_settings` 只合并设置字段，不动看守状态）、路径/Node 版本/Codex 应用校验、状态查询、进程启停、codex guard 命令、updater 配置健康检查等。

### 3.4 codex_guard.rs

Codex 配置看守（词汇与边界见 [../CONTEXT.md](../CONTEXT.md) 与 [adr/0001](adr/0001-codex-config-guard-boundaries.md)）：

- **schema 与迁移**：内置 schema 与磁盘 schema 经过版本化 envelope 合并；启动时先完成迁移前置检查，再恢复未完成事务，最后才启动 poll。迁移未决时只暂停 Guard，其他启动器功能继续可用。
- **文件格式**：每个看守文件显式使用 `toml` / `json` / `markdown` / `plain_text`；统一 bytes 校验在计划前和写后复用，旧 `md` 只作为迁移兼容值。
- **计划与所有权**：格式、参数语义、Codex 能力和路径所有权先全部预检，再按物理文件聚合；拒绝 symlink、别名、重复/父子路径、重复 TOML 路径、`file_overwrite` 冲突和 FastCtx 保留键。
- **事务边界**：`GuardCoordinator` 串行化 Guard、LauncherConfig、schema、FastCtx 和 poll 写入口。事务依次经过 Preflight、Snapshot、Writing、PostCheck、Restoring、Completed/Critical；journal、快照、durable backup 和 Launcher 状态共同提交/恢复。
- **生命周期**：`Disabled` / `Applied` / `Locked` / `Mixed` 与 `Healthy` / `Drifted` / `Invalid` / `Unsupported` / `Error` 分开建模；四动作通过一个批量 command，前端不循环旧命令。
- **多角色**：角色 ID、`agent_type` 和 `agents/<id>.toml` 主体恒等；最多托管 32 个角色，default 受保护，发现/纳入/复制/停止管理/删除均走同一事务边界。AGENTS 目录只写入转义后的角色摘要，运行证据由有界 JSONL 与双 SQLite 只读审计提供。
- **备份与审计**：写入前保存 durable backup 并按每文件 20 份保留；操作审计只存最小白名单元数据，按 30 天或 500 条清理，append 失败通过稳定错误记录暴露。
- **命令**：Guard command、DTO、event 和参数由 Rust Specta 单一来源生成到 `src/generated/guard-contracts.ts`，运行时 decoder 拒绝未知 schema/枚举；批量进度通过 `guard-operation-progress` 事件发送。

### 3.5 taskboard 集成与打包

dashi-taskboard 以 git submodule 集成于 `vendor/dashi-taskboard`（指向 fork、pin commit，决策见 [adr/0002](adr/0002-taskboard-submodule-packaging.md)，词汇见 [../CONTEXT.md](../CONTEXT.md)）：

- **构建**：`dist/web`（vite 前端产物）上游不入库，由 `beforeBuildCommand` 的 `build:taskboard` 在打包前构建；`beforeDevCommand` 只确保目录存在（编译期 tauri-build 校验资源路径）
- **打包**：`tauri.conf.json` resources 白名单只含运行时必需项（`server/ shared/ scripts/ inject/ dist/web package.json`），上游新增运行时目录需同步
- **升级**：进 submodule checkout 目标 commit，回 launcher 提交指针
- **运行时**：`get_bundled_taskboard_path` 打包后解析 `resource_dir/vendor/dashi-taskboard`，开发模式回退项目根目录；`taskboard_path` 配置可指向外部 checkout 覆盖内置版

## 4. 前端

`src/main.ts` 与 `src/theme.ts` 实现：

- 启动时加载配置并检测环境（Node 版本、Codex 应用）
- 进程状态轮询（`get_status`），按状态机渲染启停按钮
- 主题采用 tweakcn（shadcn token）的「主题族 × 模式」模型：42 族由 `scripts/build-themes.mjs` 从 tweakcn registry 生成（`src/themes.css` + `src/theme-families.ts` + `assets/fonts/` 本地字体），族和模式分别持久化到 localStorage，`<html data-theme>` 写入 `<族id>-light|dark`；系统模式监听 OS 外观变化。默认族 vercel，主题决策见 [ADR 0008](adr/0008-tweakcn-token-theming.md)
- 对话框用 `@tauri-apps/plugin-dialog`，外链走 `plugin-shell`
- 「检查更新」调 `@tauri-apps/plugin-updater`；未配置密钥时返回可读提示而非崩溃

## 5. 更新与发布

- 开发环境 updater 为空配置（pubkey/endpoints 留空），不崩溃
- 生产需生成签名密钥对、配置 endpoints，详见 [updater/SETUP.md](updater/SETUP.md)
- GitHub Release 发布流程见 [release/GITHUB_RELEASE.md](release/GITHUB_RELEASE.md)

## 6. 安全边界

- 启动器只拉起进程，不修改 `ChatGPT.app` / `app.asar`
- CDP 端口无认证，仅绑定回环地址；运行期间机器上不应执行不可信代码
- Taskboard 服务默认绑 `127.0.0.1`；LAN/云端部署的认证边界见 taskboard 侧设计文档
