# GitHub Release 发布流程

## 目标

通过推送 tag 触发 GitHub Actions，自动构建多平台安装包并创建 GitHub Release。

## 使用方式

### 1. 更新版本号

同步以下三个文件的 `version` 字段：

- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`

### 2. 创建 Release Notes

在 `release-notes/` 目录下创建 `<tag>.md` 文件，例如 `v0.1.0.md`。

### 3. 提交并推送 tag

```bash
git add -A
git commit -m "release: v0.1.0"
git tag v0.1.0
git push origin main --tags
```

### 4. 等待 GitHub Actions 完成

tag 推送后，`.github/workflows/build-release.yml` 会自动：

1. 构建 macOS / Windows / Linux 安装包（5 个并行矩阵）
2. 收集 updater 必需产物到 `updater-assets`
3. 读取 `release-notes/<tag>.md`
4. 用同一份 `release-notes/<tag>.md` 生成 `updater-assets/latest.json`
5. 组装公开的 `release-assets`
6. 创建同名 GitHub Release 并上传附件

## 前置条件

- GitHub Actions Secrets 已配置（见 [Updater 配置指南](../updater/SETUP.md)）
- 至少配置 `TAURI_UPDATER_PUBKEY`、`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- 本地能够推送到远程仓库

## 公开 Release 页面包含的文件

- `latest.json`（updater manifest）
- macOS `*aarch64*.app.tar.gz`（updater 用）
- macOS `*x64*.app.tar.gz`（updater 用）
- macOS `*aarch64*.dmg`（手动下载用）
- macOS `*x64*.dmg`（手动下载用）
- macOS `*universal*.dmg`（手动下载用）
- Windows `*-setup.exe`
- Linux `*.AppImage`
- Linux `*.deb`

`*.sig` 仅用于 manifest 生成与签名校验，不公开上传。

## 构建矩阵

| 平台 | Runner | Target | 产物 |
|---|---|---|---|
| macOS ARM | `macos-latest` | `aarch64-apple-darwin` | `.dmg` + `.app.tar.gz` + `.sig` |
| macOS Intel | `macos-latest` | `x86_64-apple-darwin` | `.dmg` + `.app.tar.gz` + `.sig` |
| macOS Universal | `macos-latest` | `universal-apple-darwin` | `.dmg` |
| Windows | `windows-latest` | - | `-setup.exe` + `.sig` |
| Linux | `ubuntu-latest` | - | `.AppImage` + `.deb` + `.sig` |

## 常见失败点

- `TAURI_UPDATER_PUBKEY` 未设置：构建时 updater 配置校验失败
- `TAURI_SIGNING_PRIVATE_KEY` 未设置：无法生成 `.sig` 签名文件
- `release-notes/<tag>.md` 不存在：release job 会直接失败
- tag 已存在：换新版本号
- Linux 构建缺少系统依赖：workflow 已自动安装，无需手动处理

## 发布后检查

- GitHub Actions：`Actions` 页面确认 `build-release` 成功
- GitHub Releases：确认 tag、release notes、附件齐全
- 本地版本文件与 release tag 一致
- 下载 `latest.json` 确认 `version` 和 `platforms` 字段正确
