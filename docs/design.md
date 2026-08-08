# Dashi Taskboard Launcher 设计文档

## 1. 概述

Tauri v2 桌面启动器，用图形界面替代手写命令，管理两个后台进程：

1. **Taskboard 服务** — dashi-taskboard 的 Node 服务（`npm start`）
2. **Codex 注入器** — 以独立 CDP 端口启动 Codex 并注入 Taskboard 面板

另集成 Tauri Updater 做应用自更新。

## 2. 架构

```
┌────────────────────────────┐
│ 前端 src/main.ts (TS + Vite) │  单页 UI：配置表单、进程状态卡片、主题切换
│  invoke() / listen()        │  状态靠轮询 get_status 刷新
└─────────────┬──────────────┘
              ▼ Tauri commands
┌─────────────────────────────┐
│ Rust 后端 src-tauri/src/     │
│  main.rs            命令入口  │
│  config.rs          配置读写  │
│  process_manager.rs 进程托管  │
└─────────────────────────────┘
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

- **schema**：内置 `guard_schema.json`（11 条 v1 参数），启动时与 `~/.dashi-taskboard-launcher/codex-guard-schema.json` 合并（同 id 内置覆盖磁盘，磁盘独有保留），UI 完全由合并结果驱动；用户可在 UI 增删自定义参数（id 前缀 `custom.`，可删除），写入该磁盘文件
- **文件列表**：看守目标文件（内置 config.toml / AGENTS.md / agents/default.toml + 自定义）存于 `LauncherConfig.codex_guard.files`；视图分组与轮询只覆盖列表内文件，路径不可重复
- **apply_mode**：`toml_key` / `toml_absent` / `file_overwrite` / `markdown_block`（`<!-- dashi:begin/end id -->` 标记区块）；TOML 读写走 `toml_edit`，保留注释与格式
- **状态**：`LauncherConfig.codex_guard`（enabled + 每参数 value/applied/locked/last_checked/last_restored）
- **轮询**：`poll_loop` tokio 任务，60s 固定间隔，仅看守文件列表内且锁定的参数；漂移即备份后改回
- **备份**：任何写入前复制目标文件到 `~/.codex/dashi-backups/`，每文件保留 20 份
- **命令**：`guard_get_view` / `guard_set_enabled` / `guard_set_value` / `guard_apply` / `guard_set_locked` / `guard_add_custom_param` / `guard_remove_custom_param` / `guard_get_schema_file_path` / `guard_get_files` / `guard_add_file` / `guard_update_file` / `guard_remove_file` / `guard_detect_file`（路径检测：只搜顶层+一层子目录，结果落盘为检测记录，之后直接读记录不重复扫）

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
- 主题采用 Tailwind v4 + daisyUI v5 的「主题族 × 模式」模型：族和模式分别持久化到 localStorage，具体主题名写入 `<html data-theme>`；系统模式监听 OS 外观变化，未配对暗色回落内置 `dark`。Geist 亮/暗 token 见 [`DESIGN.md`](../DESIGN.md) / [`DESIGN.DARK.md`](../DESIGN.DARK.md)，主题决策见 [ADR 0007](adr/0007-tailwind-daisyui-geist-theming.md)
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
