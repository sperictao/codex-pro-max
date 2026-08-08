# AGENTS.md

## 发布

- **打 tag 前必须先备齐 `release-notes/v<X.Y.Z>.md`**：`build-release.yml` 在构建完成后强制校验该文件，缺失则整个发布失败（v0.12.2 首次发布即因此返工）。正确顺序：版本号三处同步（`package.json` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml`）→ release notes → release commit → tag → 推送。
