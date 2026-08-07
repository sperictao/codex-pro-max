<p align="center">
  <img src="./assets/readme/hero.svg" width="100%" alt="Dashi Taskboard Launcher — Tauri v2 桌面启动器：Taskboard 服务托管、Codex CDP 注入、~/.codex 配置看守、应用自更新">
</p>

# Dashi Taskboard Launcher

Tauri v2 桌面启动器，用图形界面替代手写命令，一站式管理 [dashi-taskboard](https://github.com/chuspeeism/dashi-taskboard) 的使用体验：

- **Taskboard 服务** — 拉起/停止 dashi-taskboard 的 Node 服务，健康检查、状态可视化
- **Codex 注入器** — 以独立 CDP 端口启动 Codex 桌面端并注入 Taskboard 面板（macOS / Windows 商店版均可识别）
- **Codex 配置看守** — 对 `~/.codex/` 下配置文件做 schema 驱动的参数托管、锁定与漂移自动恢复（词汇与边界见 [CONTEXT.md](CONTEXT.md)）
- **应用自更新** — 内置 Tauri Updater，检查更新、下载、重启一条龙

## 下载安装

从 [Releases](https://github.com/sperictao/dashi-taskboard-launcher/releases) 下载对应平台安装包（macOS dmg / Windows setup.exe / Linux AppImage、deb）。安装后打开即可，taskboard 已打包在内，无需单独克隆。

## 仓库结构

```
dashi-taskboard-launcher/
├── src/                    前端（TS + Vite，单页 UI）
├── src-tauri/              Rust 后端（命令入口、配置、进程托管、看守、updater）
├── vendor/dashi-taskboard  git submodule → sperictao/dashi-taskboard（fork）
├── scripts/                发布辅助脚本（build-updater、generate-latest-json）
├── release-notes/          每个版本的发布说明（CI 发布时必需）
├── CONTEXT.md              领域术语表
└── docs/                   design.md、adr/、updater/、release/
```

taskboard 代码的权威来源是主仓库 `chuspeeism/dashi-taskboard`；本仓库通过 fork 的 submodule 消费它，三方关系与改动流向见 [adr/0002](docs/adr/0002-taskboard-submodule-packaging.md)。

## 开发环境

要求：Node ≥ 22.5、Rust stable、系统 Tauri 依赖（见 [Tauri 官方前置条件](https://v2.tauri.app/start/prerequisites/)）。

```bash
# submodule 必需：vendor/dashi-taskboard 是 git submodule
git clone --recurse-submodules https://github.com/sperictao/dashi-taskboard-launcher
cd dashi-taskboard-launcher
npm ci
npm run tauri dev
```

`tauri dev` 前无需手动构建 taskboard：`beforeDevCommand` 会确保资源目录存在。首次完整运行前建议先跑一次 `npm run build:taskboard`（构建 taskboard 的 web UI 到 `dist/web`），否则注入的面板没有静态资源。

## 升级内置 taskboard

```bash
cd vendor/dashi-taskboard
git fetch origin && git checkout <目标 commit/tag>   # fork main 或任意 ref
cd ../..
git add vendor/dashi-taskboard
git commit -m "chore: bump taskboard to <描述>"
```

taskboard 侧的代码改动一律在 fork 仓库里进行并推送，然后按上面流程 bump 指针；**不要**直接在 submodule 工作区改了不推（提交会随 checkout 丢失）。需要上游化的改动向 `chuspeeism/dashi-taskboard` 提 PR。

## 构建与发布

- 本地打包：`npm run tauri build`（自动先跑 `build:taskboard` 构建 taskboard web UI，再构建前端与 Rust）
- 发布：bump `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json`（及对应 lock 文件）版本号，新增 `release-notes/vX.Y.Z.md`，提交后打 tag 推送：

```bash
git tag vX.Y.Z && git push origin main vX.Y.Z
```

tag 推送触发 CI 五路构建（macOS aarch64 / x86_64 / universal、Windows、Linux）并自动创建 GitHub Release、生成 updater 的 `latest.json`。细节见 [docs/release/GITHUB_RELEASE.md](docs/release/GITHUB_RELEASE.md)；updater 密钥配置见 [docs/updater/SETUP.md](docs/updater/SETUP.md)。

## 打包说明

安装包只带 taskboard 的运行时白名单（`server/ shared/ scripts/ inject/ dist/web package.json`），web 源码、测试、文档不进安装包。上游若新增运行时目录，需同步 `src-tauri/tauri.conf.json` 的 `resources`。

## 文档索引

| 文档 | 内容 |
| --- | --- |
| [CONTEXT.md](CONTEXT.md) | 领域术语表（配置看守 + Taskboard 集成） |
| [docs/design.md](docs/design.md) | 架构与模块设计 |
| [docs/adr/0001](docs/adr/0001-codex-config-guard-boundaries.md) | 看守的生命周期与回滚边界 |
| [docs/adr/0002](docs/adr/0002-taskboard-submodule-packaging.md) | taskboard submodule 集成与打包白名单 |
| [docs/release/GITHUB_RELEASE.md](docs/release/GITHUB_RELEASE.md) | 发布流程 |
| [docs/updater/SETUP.md](docs/updater/SETUP.md) | 自更新配置 |
