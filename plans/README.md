# Codex 配置看守：分组、原子批量与多子代理角色实施计划

本计划由 `grill-with-docs` 决策会话生成，规划基线为 commit `6235f93`，
领域边界以 `CONTEXT.md` 和 ADR 0010 为准。产品源码尚未修改。

本目录替换此前围绕“仅 default 角色、无事务批量”的 001–004 草案；旧草案
与已确认的多角色、跨文件逻辑组、崩溃恢复要求冲突，不得继续执行或局部叠加。

## 最终目标

1. 每个看守文件显式声明 `toml/json/markdown/plain_text`，任何写入都经过
   格式、语义、所有权、Codex 能力和写后复查。
2. 参数按配置目的进入逻辑组，逻辑组可以跨文件；物理文件仍是一次解析、备份、
   写入和恢复的边界。
3. 全局和组级提供启用、锁定、解锁、禁用四个原子动作，绝不由前端循环旧命令。
4. 所有 Guard 写入由单一协调器串行化；预检失败零写入，写入失败恢复全批次，
   应用崩溃后先恢复未完成事务再启动轮询。
5. 最多同时托管 32 个结构化子代理角色；角色文件提供模型和推理强度，AGENTS.md
   角色目录负责选角，派生固定 `fork_turns="none"` 且不做单次模型覆盖。
6. 生命周期和健康状态分开；只有配置、能力、派生和运行证据各自满足时，界面才显示
   对应层级的正向状态，绝不声称可观测服务端内部实际路由。

## 实施顺序

| 计划 | 结果 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|
| [001](001-guard-transaction-foundation.md) | 类型化格式管线、原子存储、单写协调器、事务日志和崩溃恢复 | 无 | HIGH | IN PROGRESS |
| [002](002-logical-groups-and-bulk-lifecycle.md) | 逻辑组、生命周期枚举、全局/组级四动作、批量 DTO 与轮询接管 | 001 | HIGH | TODO |
| [003](003-multi-subagent-role-management.md) | 结构化多角色、发现/纳入/停止/删除、能力探测、角色目录与旧 default 迁移 | 002 | HIGH | TODO |
| [004](004-runtime-provenance-and-operation-audit.md) | 多角色运行时证据链、确定性归并、最小化本地审计 | 003 | HIGH | TODO |
| [005](005-guard-workbench-and-release-acceptance.md) | 工作台 UI、迁移/恢复交互、i18n、CI、文档和隔离桌面验收 | 001–004 | MED | TODO |

状态只使用 `TODO`、`IN PROGRESS`、`DONE`、`BLOCKED: <原因>`。

## 交付形态

- 使用单一堆叠分支 `codex/guard-groups-agent-roles`，后续计划从上一计划的提交继续，
  不从 `main` 重新分叉。
- 先形成一个不混功能代码的基线提交，再形成五个可审查功能提交：
  0. `chore(rust): enforce the verified 1.89 MSRV`
  1. `feat(guard): add transactional write foundation`
  2. `feat(guard): add logical groups and bulk lifecycle`
  3. `feat(guard): manage multiple subagent roles`
  4. `feat(guard): audit subagent runtime provenance`
  5. `feat(guard): ship guard workbench and migration UX`
- 未经单独授权，不 push、不创建 PR、不打 tag、不发布。

## 统一架构边界

```text
Tauri command / poll / FastCtx boundary
                │ try_lock
                ▼
         GuardCoordinator
                │
     load → validate → plan by file
                │
     journal → snapshot → atomic write
                │
       post-check → commit / restore
                │
                ▼
   versioned Launcher config + audit summary
```

- `GuardPaths` 由生产环境内部解析，Tauri 命令不接受任意根目录；测试通过内部依赖注入
  使用临时 Launcher/Codex 根目录，自动化不得触碰真实 `~/.codex`。
- 生命周期使用 `Disabled / Applied / Locked`，组和全局可派生 `Mixed`；不得继续以三个
  可产生非法组合的布尔值作为领域模型。
- 健康状态使用 `Healthy / Drifted / Invalid / Unsupported / Error`，聚合优先级固定为
  `Error > Invalid > Unsupported > Drifted > Healthy`。
- Busy/阶段/恢复是事务状态，不混入生命周期或健康状态。
- 角色 ID、`agent_type`、`agents/<id>.toml` 文件名主体三者恒等；显示名称独立。
- 后端返回稳定 code 与白名单参数，前端负责翻译；不跨边界传任意后端 prose、文件原文、
  角色指令、提示、密钥或完整用户路径。
- `GuardView`、批次报告、角色摘要和审计结果分别带 `schemaVersion`。Rust 是 Guard IPC
  合同唯一来源，生成并提交 `src/generated/guard-contracts.ts`；`npm run check:contracts`
  在临时目录重生成并逐字比较。固定 JSON fixture 同样提交进仓库，Rust 测试比较序列化
  结果，TypeScript 测试读取同一文件，阻止 DTO、command、参数键和 event 漂移。
- 单参数操作也调用同一批量引擎；旧 `engine::apply` 直写入口在计划 001 后不得存在。
- Guard 运行/暂停只控制轮询；全局“启用”是参数批量动作，两者命名和命令保持分离。
- 解锁、禁用和暂停不回滚用户配置；失败批次恢复属于事务一致性，不改变该产品语义。

## 全局质量门

每个计划先跑目标测试，再按以下顺序扩大验证：

```bash
npm run check:contracts
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-run
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --timeout 60
cargo clippy --manifest-path src-tauri/Cargo.toml --locked -- -D warnings
cargo +1.89.0 check --manifest-path src-tauri/Cargo.toml --locked
git diff --check
```

最终还必须构建 source debug `.app`，使用隔离 HOME/Codex 目录完成计划 005 的真实桌面
冒烟。新增依赖、原子替换实现和 SQLite 读取必须在 macOS、Linux、Windows 与精确 Rust
1.89.0 上通过。Plan 001 Step 0 已验证当前锁文件的真实 MSRV 为 1.89.0；任何后续依赖
更新提高这一门槛都必须显式决策，不得把 CI 改回浮动 stable 来绕过。

`rust-msrv-targeted` 不是只编译的展示矩阵。它必须在 macOS、Linux、Windows 上先
`cargo test --locked --no-run`，再通过 `scripts/run-rust-tests.mjs` 逐个执行以下精确目标，
每个目标 60 秒硬超时且 0 tests 失败：

- 001：`guard_paths_contract`、`guard_atomic_store`、`guard_transaction_recovery`；
- 002：`guard_batch_contract`、`guard_state_migration`；
- 003：`guard_roles_capability`、`guard_role_migration`、`guard_command_registry`；
- 004：`guard_runtime_audit`、`guard_operation_audit`。

stable Rust 的全量测试和 Clippy 仍独立保留；矩阵不得用单个平台或模块过滤器代替上述
公开边界目标。

## 本期不做

- 原始 TOML 编辑器、云同步、远程配置下发。
- Codex++ 或第三方 provider 管理、Codex 自动安装或升级。
- 自动执行、评测或调度子代理任务。
- 修改现有 Taskboard/FastCtx 服务启动逻辑。
- 通过 CDP 修改请求，或声称能证明服务端内部实际模型路由。
