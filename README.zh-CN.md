<div align="center">

<img src="./assets/readme/hero.zh-CN.svg" width="100%" alt="Dashi Taskboard Launcher — Tauri v2 桌面启动器：Taskboard 服务托管、Codex CDP 注入、~/.codex 配置看守、应用自更新">

**图形界面替代手写命令，一站式管理 dashi-taskboard 的使用体验。**

[![GitHub Release](https://img.shields.io/github/v/release/sperictao/dashi-taskboard-launcher)](https://github.com/sperictao/dashi-taskboard-launcher/releases)
[![Tauri 2](https://img.shields.io/badge/Tauri-2.x-FFC131?logo=tauri&logoColor=white)](https://tauri.app)
[![TypeScript](https://img.shields.io/badge/TypeScript-5.x-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![Rust](https://img.shields.io/badge/Rust-stable-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org)

[English](README.md) · [简体中文](README.zh-CN.md)

</div>

---

## ✨ 功能亮点

- 🟢 **Taskboard 服务** — 拉起/停止 [dashi-taskboard](https://github.com/chuspeeism/dashi-taskboard) 的 Node 服务，健康检查、状态可视化
- 💉 **Codex 注入器** — 以独立 CDP 端口启动 Codex 桌面端并注入 Taskboard 面板（macOS / Windows 商店版均可识别）
- 🔒 **Codex 配置看守** — 对 `~/.codex/` 下配置文件做 schema 驱动的参数托管、锁定与漂移自动恢复（词汇与边界见 [CONTEXT.md](CONTEXT.md)）
- 🔄 **应用自更新** — 内置 Tauri Updater，检查更新、下载、重启一条龙

---

## 📦 下载安装

从 [Releases](https://github.com/sperictao/dashi-taskboard-launcher/releases) 下载对应平台安装包（macOS dmg / Windows setup.exe / Linux AppImage、deb）。安装后打开即可，taskboard 已打包在内，无需单独克隆。

---

## 🧩 工作原理

1. **拉起服务** — 启动打包在内的 taskboard Node 服务，健康检查通过后标记就绪
2. **注入面板** — 以独立 CDP 端口拉起 Codex 桌面端，把 Taskboard 面板注入其界面
3. **看守配置** — 按 schema 托管 `~/.codex/` 参数；锁定后轮询（60s），发现漂移自动改回（写入前备份）
4. **自我更新** — 检查 GitHub Releases 的 `latest.json`，下载、验签、重启完成升级

---

## 🚀 开发环境

要求：Node ≥ 22.5、Rust stable、系统 Tauri 依赖（见 [Tauri 官方前置条件](https://v2.tauri.app/start/prerequisites/)）。

```bash
# submodule 必需：vendor/dashi-taskboard 是 git submodule
git clone --recurse-submodules https://github.com/sperictao/dashi-taskboard-launcher
cd dashi-taskboard-launcher
npm ci
npm run tauri dev
```

`tauri dev` 前无需手动构建 taskboard：`beforeDevCommand` 会确保资源目录存在。首次完整运行前建议先跑一次 `npm run build:taskboard`（构建 taskboard 的 web UI 到 `dist/web`），否则注入的面板没有静态资源。

---

## 🏗️ 仓库结构

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

---

## 🔄 升级内置 taskboard

```bash
cd vendor/dashi-taskboard
git fetch origin && git checkout <目标 commit/tag>   # fork main 或任意 ref
cd ../..
git add vendor/dashi-taskboard
git commit -m "chore: bump taskboard to <描述>"
```

taskboard 侧的代码改动一律在 fork 仓库里进行并推送，然后按上面流程 bump 指针；**不要**直接在 submodule 工作区改了不推（提交会随 checkout 丢失）。需要上游化的改动向 `chuspeeism/dashi-taskboard` 提 PR。

---

## 🚢 构建与发布

- 本地打包：`npm run tauri build`（自动先跑 `build:taskboard` 构建 taskboard web UI，再构建前端与 Rust）
- 发布：bump `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json`（及对应 lock 文件）版本号，新增 `release-notes/vX.Y.Z.md`，提交后打 tag 推送：

```bash
git tag vX.Y.Z && git push origin main vX.Y.Z
```

tag 推送触发 CI 五路构建（macOS aarch64 / x86_64 / universal、Windows、Linux）并自动创建 GitHub Release、生成 updater 的 `latest.json`。细节见 [docs/release/GITHUB_RELEASE.md](docs/release/GITHUB_RELEASE.md)；updater 密钥配置见 [docs/updater/SETUP.md](docs/updater/SETUP.md)。

> **打包说明**：安装包只带 taskboard 的运行时白名单（`server/ shared/ scripts/ inject/ dist/web package.json`），web 源码、测试、文档不进安装包。上游若新增运行时目录，需同步 `src-tauri/tauri.conf.json` 的 `resources`。

---

## 🛠️ 技术栈

| 层 | 技术 |
| --- | --- |
| 桌面框架 | Tauri 2.x（Rust） |
| 前端 | TypeScript 5 + Vite 6（单页 UI） |
| taskboard 集成 | git submodule（fork 仓库消费上游） |
| 配置看守 | schema 驱动，TOML / Markdown 区块 / 整文件三种比对模式 |
| 自更新 | Tauri Updater + GitHub Releases |

---

## 📜 常用脚本

```bash
npm run tauri dev          # 开发模式（前端 + Rust 后端）
npm run tauri build        # 生产打包
npm run build              # 仅构建前端（tsc + vite build）
npm run build:taskboard    # 构建内置 taskboard 的 web UI 到 dist/web
npm run build:updater      # 生成 updater 产物
```

---

## 📚 文档索引

| 文档 | 内容 |
| --- | --- |
| [CONTEXT.md](CONTEXT.md) | 领域术语表（配置看守 + Taskboard 集成 + FastCtx 集成） |
| [docs/design.md](docs/design.md) | 架构与模块设计 |
| [docs/adr/0001](docs/adr/0001-codex-config-guard-boundaries.md) | 看守的生命周期与回滚边界 |
| [docs/adr/0002](docs/adr/0002-taskboard-submodule-packaging.md) | taskboard submodule 集成与打包白名单 |
| [docs/adr/0003](docs/adr/0003-fastctx-delegate-to-cli.md) | FastCtx 集成委托 fastctx CLI 的决策 |
| [docs/release/GITHUB_RELEASE.md](docs/release/GITHUB_RELEASE.md) | 发布流程 |
| [docs/updater/SETUP.md](docs/updater/SETUP.md) | 自更新配置 |

---

## 📄 第三方声明

- [dashi-taskboard](https://github.com/chuspeeism/dashi-taskboard) — 内置的任务看板，以 git submodule 集成于 `vendor/dashi-taskboard` 并随安装包分发（见 [ADR 0002](docs/adr/0002-taskboard-submodule-packaging.md)）。上游未声明许可；打包遵循 [CONTEXT.md](CONTEXT.md) 所述的上游 → fork（`sperictao/dashi-taskboard`）→ PR 协作流。启动器侧集成代码为我们自己的工作；看板本体为上游作者的作品。
- [FastCtx](https://github.com/yc-duan/fastctx) — 可选集成，采用 [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0) 许可。本启动器**不**再分发、不内嵌 FastCtx，仅在运行时调用用户自行安装的 `fastctx` CLI。本仓库中的全部集成代码均为我们自己的工作与独自的责任，不代表 FastCtx 作者的认可，FastCtx 作者亦不承担由此产生的任何责任。FastCtx 内嵌 Pdfium，其第三方许可见 FastCtx 的 `THIRD_PARTY_LICENSES.md`（仅在再分发 FastCtx 二进制时相关）。

---

<div align="center">

Made with ❤️ by [Eric Tao](https://github.com/sperictao)

</div>
