# Taskboard 以 git submodule 集成，打包白名单裁剪

dashi-taskboard（见 CONTEXT.md "Taskboard 集成"域）从 vendor 纯文件拷贝改为 **git submodule**，指向 fork（`sperictao/dashi-taskboard`），pin 具体 commit，launcher 发版时显式 bump 指针。Tauri `resources` 从整目录映射改为**运行时白名单**：`server/ shared/ scripts/ inject/ dist/web package.json`。

原拷贝方式在 v0.2.5 暴露了结构性脱节：注入器修复直接埋在 vendor 快照里，上游（主仓库 `chuspeeism/dashi-taskboard`）一无所知，launcher 与 taskboard 也无法各自更新。submodule 是"各自更新 + launcher pin 版本"的标准语义；指向 fork 而非主仓库，是因为 launcher 侧的修复要等 PR 被合并才出现在主仓库，挂 fork 不阻塞自己的发版节奏。

`dist/web`（vite 前端构建产物，上游 gitignore）是运行时必需但 submodule checkout 里没有的东西，由 launcher 构建管线补齐：`beforeBuildCommand` 先跑 `pnpm run build:taskboard`（submodule 内 install + build:web），CI checkout 用 `submodules: recursive`。

白名单的直接诱因是整目录映射曾把 `.data/`（运行时 sqlite）和 `dist/` 打进安装包；submodule 化后 `web/` 源码、`test/`、proof 图片进来问题更大。代价是上游若新增运行时目录，launcher 会静默漏打——接受这个代价，白名单比黑名单更不容易把新垃圾带进安装包。

**Consequences**：clone 需要 `git clone --recurse-submodules`（或 `git submodule update --init`）；升级 taskboard = 进 submodule 拉到目标 commit + launcher 提交指针 + 发版；对 taskboard 的代码改动必须在 fork 仓库里进行、经 PR 上游化，直接在 submodule 工作区改而不推送会丢。
