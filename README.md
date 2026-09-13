<div align="center">

<img src="./assets/readme/hero.svg" width="100%" alt="Codex Pro Max — Tauri v2 desktop launcher for the Codex development environment: taskboard service, CDP panel injection, ~/.codex config guard and model config, Taskboard skill, FastCtx, themes, command palette, and self-updates">

**A GUI that replaces hand-typed commands — the whole dashi-taskboard experience in one desktop app.**

[![GitHub Release](https://img.shields.io/github/v/release/sperictao/codex-pro-max)](https://github.com/sperictao/codex-pro-max/releases)
[![Tauri 2](https://img.shields.io/badge/Tauri-2.x-FFC131?logo=tauri&logoColor=white)](https://tauri.app)
[![TypeScript](https://img.shields.io/badge/TypeScript-5.x-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![Rust](https://img.shields.io/badge/Rust-stable-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[English](README.md) · [简体中文](README.zh-CN.md)

</div>

> **Note:** This project was formerly named **Dashi Taskboard Launcher** at `sperictao/dashi-taskboard-launcher`, and has been renamed to **Codex Pro Max**. Existing clones can update their remote with:
> `git remote set-url origin https://github.com/sperictao/codex-pro-max.git`

---

## ✨ Highlights

- 🟢 **Taskboard Service** — start/stop the bundled [dashi-taskboard](https://github.com/chuspeeism/dashi-taskboard) Node service with health checks and an aggregate status indicator on the home page; dashboard, issues, list, and Gantt views included
- 💉 **Codex Injector** — launch Codex on a dedicated CDP port and inject the Taskboard panel into its UI (macOS and Windows Store builds)
- 🔒 **Codex Config Guard** — schema-driven parameter management, locking, and automatic drift recovery for config files under `~/.codex/` (terminology and boundaries in [CONTEXT.md](CONTEXT.md))
- 🧠 **Model Config** — visually manage the active model, model providers (custom base_url and auth), and model presets in `~/.codex/config.toml` with one-click switching (inspired by CCursor)
- 🧩 **Taskboard Skill** — installs the bundled `manage-taskboard` skill into Codex in one click (a symlink at `~/.codex/skills/manage-taskboard`), so Codex can create, view, and manage board tasks itself; the link stays in sync with taskboard updates
- 🧰 **FastCtx Integration** — one-click install of the [FastCtx](https://github.com/yc-duan/fastctx) MCP runtime and integrate/unapply it into Codex, delegated to the `fastctx` CLI
- 🎨 **Themes** — 42 tweakcn theme families with native light / dark / system modes; 26 UI font families self-hosted in-app, fully offline
- 🌐 **Bilingual UI** — English and 中文, following the system language by default and switchable at runtime
- ⌘ **Command Palette** — `Cmd/Ctrl+K` (or the header search button) fuzzy-finds and jumps straight to any view, settings section, or action: check for updates, open the repository, open the log directory, toggle light/dark
- 🔄 **Self-Update** — built-in Tauri Updater: check, download, restart, done
- 🖥️ **Desktop-Native Behavior** — launch at login, minimize to tray, single-instance enforcement, and a system notification when a managed process dies

---

## 📦 Download & Install

Grab the installer for your platform from [Releases](https://github.com/sperictao/codex-pro-max/releases) (macOS dmg / Windows setup.exe / Linux AppImage, deb). Open it after install — taskboard is bundled inside, no separate clone needed.

---

## 🧩 How It Works

1. **Start the service** — launches the bundled taskboard Node service and marks it ready once the health check passes
2. **Inject the panel** — starts Codex on a dedicated CDP port and injects the Taskboard panel into its UI
3. **Guard the config** — manages `~/.codex/` parameters per schema; while locked, polling (60s) detects drift and restores the configured value, backing up to `~/.codex/dashi-backups/` before every write
4. **Update itself** — checks `latest.json` on GitHub Releases, downloads, verifies, and restarts

---

## 🚀 Development

Requirements: Node ≥ 22.5, Rust stable, and the system Tauri dependencies (see [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)).

```bash
# taskboard submodule required
git clone --recurse-submodules https://github.com/sperictao/codex-pro-max
cd codex-pro-max
pnpm install
pnpm run tauri dev
```

Before the first full run, it is still recommended to run `pnpm run build:taskboard` once (builds the taskboard web UI into `vendor/dashi-taskboard/dist/web`), otherwise the injected panel has no static assets.

Checks: `pnpm test` runs the craft-spec gate, the theme tests, and the Vitest suites; `cargo test --manifest-path src-tauri/Cargo.toml` covers the Rust side; `pnpm smoke` drives the real UI in headless Chromium (run `pnpm exec playwright install chromium-headless-shell` once first).

---

## 🏗️ Repository Layout

```
codex-pro-max/
├── src/                    frontend (React 19 + TS + Vite, single-page UI)
├── src-tauri/              Rust backend (commands, config, process hosting, guard, updater)
├── vendor/dashi-taskboard  git submodule → sperictao/dashi-taskboard (fork)
├── assets/                 self-hosted UI fonts (generated) and README hero art
├── scripts/                build & release helpers (themes, craft gate, release preflight, smoke, updater)
├── release-notes/          per-version release notes (required by CI releases)
├── CONTEXT.md              domain glossary
└── docs/                   design.md, craft-spec.md, adr/, updater/, release/
```

The authoritative source of taskboard code is the upstream repo `chuspeeism/dashi-taskboard`; this repo consumes it through a fork submodule. See [adr/0002](docs/adr/0002-taskboard-submodule-packaging.md) for the taskboard relationship and change flow.

---

## 🔄 Upgrading the Bundled taskboard

```bash
cd vendor/dashi-taskboard
git fetch origin && git checkout <target commit/tag>   # fork main or any ref
cd ../..
git add vendor/dashi-taskboard
git commit -m "chore: bump taskboard to <description>"
```

Make taskboard-side changes in the fork repo and push them first, then bump the pointer as above; **never** commit directly inside the submodule worktree without pushing (commits are lost on checkout). Changes that belong upstream should go to `chuspeeism/dashi-taskboard` as a PR.

---

## 🚢 Build & Release

- Local bundle: `pnpm run tauri build` (builds taskboard, then builds the frontend and Rust)
- Release: bump the version in `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` (plus lock files), add `release-notes/vX.Y.Z.md`, then run the preflight — it checks that the three version files agree, that the tag matches the version, that the release notes exist, and that every git-tracked bundle resource path is present (the same script runs as the CI `validate` job, ahead of the build matrix):

```bash
pnpm run check:release -- --tag vX.Y.Z
```

- Then commit, tag, and push:

```bash
git tag vX.Y.Z && git push origin main vX.Y.Z
```

Pushing the tag triggers five CI builds (macOS aarch64 / x86_64 / universal, Windows, Linux), creates the GitHub Release automatically, and generates the updater `latest.json`. Details in [docs/release/GITHUB_RELEASE.md](docs/release/GITHUB_RELEASE.md); updater key setup in [docs/updater/SETUP.md](docs/updater/SETUP.md).

> **Packaging note**: installers ship the taskboard runtime whitelist. Plugin source trees, tests, and development dependencies are excluded. If a runtime resource changes, keep `src-tauri/tauri.conf.json` in sync.

---

## 🛠️ Tech Stack

| Layer | Technology |
| --- | --- |
| Desktop framework | Tauri 2.x (Rust) |
| Frontend | React 19 + TypeScript 5 + Vite 8 (single-page UI, [ADR 0010](docs/adr/0010-shell-frontend-react-rewrite.md)) |
| State and i18n | Zustand store fed straight from Tauri events; react-i18next with en / zh-CN dictionaries |
| UI and theming | Tailwind CSS v4 + tweakcn (shadcn token) theme system; 42 families, 26 self-hosted fonts ([ADR 0008](docs/adr/0008-tweakcn-token-theming.md)) |
| Component layer | in-repo shadcn `radix-nova` primitives governed by a machine-checked [craft spec](docs/craft-spec.md) ([ADR 0011](docs/adr/0011-shell-component-primitive-layer.md)) |
| taskboard integration | git submodule (consuming upstream via a fork) |
| Config guard | schema-driven; TOML key / Markdown block / whole-file comparison modes |
| FastCtx integration | delegated to the `fastctx` CLI (one-click npm global install in Settings) |
| Self-update | Tauri Updater + GitHub Releases |

---

## 📜 Common Scripts

```bash
pnpm run tauri dev          # dev mode (frontend + Rust backend)
pnpm run tauri build        # production bundle
pnpm run build              # frontend only (tsc + vite build)
pnpm test                   # craft-spec gate + theme tests + Vitest suites
pnpm run smoke              # headless-Chromium UI smoke suite
pnpm run check:release -- --tag vX.Y.Z   # release preflight
pnpm run build:taskboard    # build the bundled taskboard web UI
pnpm run build:updater      # generate updater artifacts
```

---

## 📚 Documentation

| Doc | Contents |
| --- | --- |
| [CONTEXT.md](CONTEXT.md) | Domain glossary (config guard, taskboard integration, i18n, FastCtx, model config, shell, theming, component layer) |
| [docs/design.md](docs/design.md) | Architecture and module design |
| [docs/craft-spec.md](docs/craft-spec.md) | Shell UI craft spec, enforced by `scripts/check-craft.mjs` |
| [scripts/build-themes.mjs](scripts/build-themes.mjs) | Theme build: tweakcn registry → tokens + local fonts |
| [docs/adr/0011](docs/adr/0011-shell-component-primitive-layer.md) | In-repo component primitive layer (shadcn `radix-nova` recipe) |
| [docs/adr/0010](docs/adr/0010-shell-frontend-react-rewrite.md) | Shell frontend rewritten in React 19 (supersedes ADR 0009) |
| [docs/adr/0008](docs/adr/0008-tweakcn-token-theming.md) | tweakcn token theming (supersedes daisyUI ADR 0007) |
| [docs/adr/0001](docs/adr/0001-codex-config-guard-boundaries.md) | Guard lifecycle and rollback boundaries |
| [docs/adr/0002](docs/adr/0002-taskboard-submodule-packaging.md) | taskboard submodule integration and packaging whitelist |
| [docs/adr/0003](docs/adr/0003-fastctx-delegate-to-cli.md) | FastCtx integration delegates to the fastctx CLI |
| [docs/release/GITHUB_RELEASE.md](docs/release/GITHUB_RELEASE.md) | Release pipeline |
| [docs/updater/SETUP.md](docs/updater/SETUP.md) | Self-update configuration |

---

## 📄 Third-Party Notices

- [dashi-taskboard](https://github.com/chuspeeism/dashi-taskboard) — the bundled task board, integrated as a git submodule at `vendor/dashi-taskboard` and shipped inside the installer (see [ADR 0002](docs/adr/0002-taskboard-submodule-packaging.md)). Upstream declares no license; bundling follows the upstream → fork (`sperictao/dashi-taskboard`) → PR workflow described in [CONTEXT.md](CONTEXT.md). Launcher-side integration code is our own work; the taskboard itself remains the upstream author's work.
- [FastCtx](https://github.com/yc-duan/fastctx) — optional integration, licensed under [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0). This launcher does **not** redistribute or embed FastCtx; it invokes a user-installed `fastctx` CLI at runtime. All integration code in this repository is our own work and our sole responsibility; it is not endorsed by the FastCtx authors, who bear no liability for it. FastCtx embeds Pdfium — see FastCtx's `THIRD_PARTY_LICENSES.md` (relevant only when redistributing FastCtx binaries).
- UI fonts — 26 Google Fonts families (latin / latin-ext subsets) self-hosted inside the app, fetched from the tweakcn registry by [scripts/build-themes.mjs](scripts/build-themes.mjs); each family's license (mostly OFL) is on its Google Fonts page.

---

## 📄 License

Released under the [MIT License](LICENSE) — © 2026 Eric Tao. The license covers this launcher's own code; bundled third-party components keep their own terms (see Third-Party Notices above).

---

## 🔗 Friendly Links

- [Linux.do](https://linux.do) — developer community forum

---

<div align="center">

Made with ❤️ by [Eric Tao](https://github.com/sperictao)

</div>
