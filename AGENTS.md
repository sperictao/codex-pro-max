# AGENTS.md

## 发布

- **打 tag 前必须先备齐 `release-notes/v<X.Y.Z>.md`**：`build-release.yml` 在构建完成后强制校验该文件，缺失则整个发布失败（v0.12.2 首次发布即因此返工）。正确顺序：版本号三处同步（`package.json` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml`）→ release notes → release commit → tag → 推送。
- **打 tag 前跑 `npm run check:release -- --tag v<X.Y.Z>`**：校验版本号三处一致 + tag 与版本一致 + release notes 存在。同一脚本在 CI `validate` job 中秒级运行（构建矩阵之前），本地先跑一遍可以零成本拦截发布失败。
