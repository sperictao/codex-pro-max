# 合并上游更新记录（2026-08-08）

## 背景

- 本仓库是 `chuspeeism/dashi-taskboard` 的 fork（远程 `upstream`），自有远程为 `origin`（`sperictao/dashi-taskboard`）。
- 本地 `main` 有两个上游未合并的提交，且已推送到 `origin/main`：
  - `7e6249a` fix(injector): recognize windows codex/chatgpt page targets
  - `c8cc067` fix(injector): wait for renderer target and page load before injecting, add retries and logs
- 上游 `upstream/main` 有 100+ 个新提交需要拉入。

## 决策：用 merge，不用 rebase

两个本地提交已经推送到 `origin/main`。rebase 会把这两个提交改写成新提交，必须 force-push 才能更新 origin，有覆盖风险；merge 不改写历史，两个提交原样保留，普通 push 即可。代价是历史里多一个合并提交，可接受。

## 操作步骤

```bash
git fetch upstream
git merge upstream/main --no-edit   # 产生 1 处冲突
# 解决 scripts/codex-injector.mjs 冲突（见下节）
node --test test/codex-targets-filter.test.mjs   # 验证
git add scripts/codex-injector.mjs
git commit --no-edit                # 合并提交 a20eac1
git push origin main                # 7e6249a..a20eac1，无需 force
```

## 冲突解决：scripts/codex-injector.mjs

冲突位置：`codexTargets()` 中页面目标的过滤条件。

| | 上游版本 | 本地版本（保留） |
|---|---|---|
| 排除路由 | URL 字符串匹配 `initialRoute=` 的 global-dictation、avatar-overlay | `isExcludedCodexRoute()` 解析路由，额外排除 quick-chat 各路由 |
| 正向匹配 | `app://` 前缀或 `title === "Codex"` | `app://` 前缀或 `isCodexPageTarget()`（title+url 含 codex，或 ChatGPT 桌面页） |

**决定：保留本地版本。** 本地版本是上游过滤逻辑的严格超集——上游要排除的两个路由本地都覆盖，且本地还多了 quick-chat 排除和 Windows 版 Codex/ChatGPT 桌面页识别（这正是 `7e6249a` 的修复内容）。若采用上游版本等于回退这两个修复。

注意：上游把部分逻辑抽到了新文件 `scripts/codex-injector-runtime.mjs`，但 `codexTargets()` 仍留在 `codex-injector.mjs` 内，本次冲突与它无关。

## 验证

- `grep '^<<<<<<<\|^>>>>>>>' scripts/` 无残留冲突标记
- `node --test test/codex-targets-filter.test.mjs` 全部通过（fail 0）
- 合并后 `git log` 确认 `7e6249a`、`c8cc067` 仍在历史中

## 后续同步上游的推荐做法

```bash
git fetch upstream
git merge upstream/main --no-edit
# 如 codex-injector.mjs 再次冲突，一般仍保留本地过滤逻辑
git push origin main
```

仅当两个修复被上游以等价形式吸收后，才可考虑改用 rebase 保持线性历史。
