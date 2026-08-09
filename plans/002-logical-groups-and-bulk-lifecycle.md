# Plan 002：实现逻辑分组和全局/组级批量生命周期

> 执行者：只在 Plan 001 完成且事务故障注入全绿后开始。批量动作必须调用一个后端
> 命令；禁止前端 `Promise.all` 或逐参数循环拼出“批量”。

## 状态

- 优先级：P0
- 工作量：L
- 风险：HIGH
- 依赖：Plan 001
- 分支：`codex/guard-groups-agent-roles`

## 根因与不变量

`guard_schema.json` 虽包含 `group`，`GuardParam` 在 `mod.rs:28-59` 并未反序列化；
`view.rs:57-98` 实际按物理 `GuardFile` 分组。参数状态又由 `applied/locked` 两个布尔值
拼接，能反序列化出 `locked=true, applied=false`。

完成后：

> 逻辑组只表达配置目的，文件只表达 I/O 边界；生命周期是合法枚举。全局、组、参数
> 操作都展开成同一原子批次，聚合状态从成员派生，不持久化第二份组状态。

主要文件：`model.rs`、新增 `migration.rs`/`batch.rs`、`commands.rs`、`poll.rs`、
`view.rs`、`schema.rs`、`files.rs`、`guard_schema.json`、`main.rs`、生成合同与迁移/批次
fixtures；新增精确 integration targets：`src-tauri/tests/guard_batch_contract.rs`、
`src-tauri/tests/guard_state_migration.rs`。

## Step 1：版本化持久模型和旧状态迁移

新增/调整类型：

```rust
enum ParameterLifecycle { Disabled, Applied, Locked }
enum LifecycleSummary { Disabled, Applied, Locked, Mixed }
enum HealthStatus { Healthy, Drifted, Invalid, Unsupported, Error }

struct GuardGroup {
    id: String,
    name: LocalizedLabel,
    builtin: bool,
    order: u32,
}
```

- `CodexGuardState` 和磁盘 schema 使用 `schema_version = 1` envelope；不再保存裸数组。
- `GuardParam` 增加稳定 `group_id` 和物理 `file_id`，不再让 `file` 字符串同时承担身份。
- v0 状态迁移：`locked=true && applied=true → Locked`，仅 applied → Applied，均 false →
  Disabled；`locked=true && applied=false` 标记迁移 Invalid，禁止轮询/写入，等待显式选择
  Disabled 或重新启用，不静默修正。
- 非法生命周期进入独立 `PendingLifecycleMigration`，提供
  `guard_lifecycle_migration_resolve(parameter_id, disabled|apply)`：选择 Disabled 只修复
  Launcher 状态，选择 Apply 则走完整实时预检/批次；Plan 005 必须提供对应向导。
- 现有内置映射固定为：
  - `subagent-optimization`：全部 multi_agent_v2、agents-v1-remove、AGENTS 托管区块以及
    Plan 003 的全部角色；
  - `general`：image_generation 等非子代理内置参数；
  - `uncategorized`：仅承接无组旧自定义参数，固定最后、为空时隐藏。
- 自定义组接在内置组之后；内置组 ID、名称和顺序不可修改。
- 旧 GuardFile 的 `md` 迁移为 `markdown`；缺失/未知格式仅在内置路径或扩展名唯一对应时
  自动推断，否则进入 `PendingFormatMigration`，选择前该文件停写。提供显式格式选择，
  不把未知扩展名默认为 plain text。
- 迁移先生成计划并备份旧 Launcher 配置；失败保持 v0 字节和 Guard 停写，其他功能继续。

测试：所有 v0 布尔组合、旧 `group=file` 值、缺组自定义参数、重复 ID、迁移幂等、保存
中断、降级备份存在。

## Step 2：定义批量作用域、动作与报告合同

```rust
enum BatchScope {
    All,
    Group { group_id: String },
    Parameter { parameter_id: String },
    Role { role_id: String },
}
enum BatchAction { Apply, Lock, Unlock, Disable }

struct BatchRequest { schema_version: u32, scope: BatchScope, action: BatchAction }
struct BatchReport {
    schema_version: u32,
    batch_id: String,
    outcome: BatchOutcome,
    changed: u32,
    unchanged: u32,
    files: u32,
    diagnostics: Vec<ValidationDiagnostic>,
}
```

`BatchOutcome` 只允许 `Committed / Rejected / RolledBack / CriticalRecovery`。业务操作不返回
“部分成功”；成员只能 changed/unchanged。恢复失败才是 CriticalRecovery。

动作规范：

| 动作 | 成员规则 |
|---|---|
| Apply | Disabled 与漂移的 Applied 成员写入；Locked 保持不变 |
| Lock | Disabled/Applied 先启用再锁定；Locked 保持不变 |
| Unlock | 只把 Locked 变为 Applied；其他不变，不写目标文件 |
| Disable | Locked 先解锁，再把所有成员变为 Disabled；不回滚目标值 |

无变化是成功的 `unchanged`。eligibility 按动作区分：Invalid、Unsupported、Error 只阻塞
Apply/Lock；Unlock/Disable 必须始终可解除现有生命周期，只在 Launcher 状态不可读、
事务恢复中或所有权不确定时拒绝。无关组仍可执行；全局 Apply/Lock 遇到任何阻塞成员
整体拒绝。

新增只读 `guard_preview_batch(request) -> BatchPreview`，返回 preview ID、配置 revision、
affectedMembers、affectedFiles、changed/unchanged 预估和 blockers。全局 Apply/Lock UI 先
preview 再确认；execute 必须重新验证 preview revision/hash，过期返回 `preview_stale`。

所有新增 DTO、command 和 event 进入 Plan 001 的生成合同；提交的 JSON fixture 锁定
camelCase 字段、snake_case enum 和未知版本拒绝，禁止新增手写 invoke。

## Step 3：实现按文件聚合的批次计划器

- scope 先解析成稳定排序的成员集合，再按规范化 `file_id` 聚合。
- 对所有文件完成格式、语义、所有权和状态预检后，才调用 Plan 001 事务执行器。
- 同一文件所有参数一次解析/合并；不同逻辑组共享文件时仍只写一次。
- 状态提交和目标文件写入属于 Plan 001 定义的同一 journal 事务参与者；状态保存或 commit
  marker 失败必须同时恢复目标文件与 Launcher lifecycle。
- `Unlock/Disable` 若不改目标文件，也必须经协调器原子更新 Launcher 状态，避免 poll 用旧
  快照覆盖。
- 旧单参数 apply/lock/unlock/disable 命令改为薄适配器或删除；不能保留第二套状态机。

新增 `batch.rs` 及 table-driven 状态机测试，覆盖组跨三文件、同文件多组、空 scope、
全部 unchanged、第 N 文件失败、状态提交失败、post-check 失败和恢复失败。

## Step 4：让 poll 服从新生命周期和事务边界

- `poll.rs` 只选择 `Locked` 成员；非法/迁移未决/Critical 文件不进入计划。
- poll 使用协调器 `try_lock`，忙时记录安全计数并跳过本轮，不排队。
- 一次 poll 按“共享同一物理文件”的连通分量拆分恢复批次；每个分量内全量预检、原子
  执行，互不共享文件的无关组不会因另一组持续失败而反复回滚。
- `last_checked` 仅在实际检查后更新；`last_restored` 仅在 post-check Healthy 后更新。
- 错误 code 持久化；成功的相应检查只清除同来源错误，不清除迁移/恢复等无关错误。
- Guard 运行暂停不隐藏页面、不改变任何成员生命周期。

## Step 5：实现逻辑组命令和视图 DTO

新增命令：

- `guard_execute_batch(request)`
- `guard_preview_batch(request)`
- `guard_group_create(name)`、`guard_group_rename(id, name)`
- `guard_group_reorder(ids)`、`guard_group_delete(id)`
- `guard_parameter_move(parameter_id, group_id)`
- `guard_lifecycle_migration_resolve(parameter_id, choice)`
- `guard_file_format_migration_resolve(file_id, format)`

规则：自定义组名 UTF-8、去首尾空白、1–80 字符、无 NUL；大小写不敏感重名拒绝；
非空组、内置组不可删除；只有无 Guard 写事务时可移动自定义参数。内置参数归组固定。

`GuardView v2` 分开返回：

- `runtimeState`（Running/Suspended）；
- 全局生命周期/健康、动作 eligibility、reason code 与影响计数；
- `groups[]`（逻辑组、文件引用、成员、双状态轴、动作 eligibility 与影响计数）；
- `files[]`（物理格式、路径、文件诊断）；
- 当前 batch/recovery 摘要。

前端不重算业务 eligibility。Role scope 在 Plan 003 接入实际成员；本计划先冻结合同并用
fixture role member 验证它可同时覆盖角色文件和 AGENTS 目录。聚合优先级、Mixed、影响
计数和错误隔离全部由 Rust 单测证明。

在 `main.rs:925-964` 注册统一批次和组命令；通过 Tauri event 发送
`guard-operation-progress { batchId, phase, completed, total }`，事件不包含路径或值。

## Step 6：TDD 验收矩阵

至少新增：

- 四动作 × 三生命周期的完整表；
- 全局/组/参数/角色四种 scope；
- 跨文件组、同文件跨组、空组、Uncategorized；
- Healthy/Drifted/Invalid/Unsupported/Error 聚合优先级；
- busy 时用户写请求拒绝、poll 跳过；
- Invalid/Unsupported/Error 下 Unlock/Disable 可用、Apply/Lock 阻塞；
- 生命周期迁移两种 resolve 与 stale preview；
- format 自动推断/待选择/四种显式 resolve；
- 批次前后配置和目标文件字节断言；
- DTO fixture 与旧配置反序列化。

`guard_batch_contract` 必须从公开 command/DTO 边界覆盖四种 scope、四动作、preview stale、
单文件一次写入和全批次回滚；`guard_state_migration` 必须用真实 v0 配置字节覆盖生命周期、
group/file 分离、格式待选择、升级备份和迁移中断恢复。内部模块单测不能替代这两个目标。

## 验证

```bash
npm run check:contracts
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_batch_contract --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_state_migration --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::migration --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::batch --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::poll --timeout 60
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-run
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --timeout 60
cargo clippy --manifest-path src-tauri/Cargo.toml --locked -- -D warnings
cargo +1.89.0 check --manifest-path src-tauri/Cargo.toml --locked
npm test
npm run build
git diff --check
```

Plan 001 的 `rust-msrv-targeted` matrix 在本提交扩展为三平台实际执行
`guard_batch_contract` 与 `guard_state_migration`；0 tests、仅编译或只在 stable Linux
运行均不得视为本计划通过。

## Done

- [ ] 逻辑组与物理文件在模型和 DTO 中完全分离。
- [ ] 生命周期不再能表示 locked-but-disabled 等非法组合。
- [ ] 全局/组级四动作全部通过一个后端批次命令。
- [ ] Role scope、预览计数和生命周期迁移解除入口已冻结并有合同测试。
- [ ] Invalid/Unsupported/Error 不会锁死 Unlock/Disable。
- [ ] 预检失败零写入，事务失败无业务部分成功。
- [ ] poll、单参数命令和批量命令共享计划器与协调器。
- [ ] 内置/自定义/未分类组迁移、排序、删除规则均有测试。
- [ ] 双状态轴及 eligibility 由后端权威派生。
- [ ] 两个公开边界 integration targets 在三平台与最终 MSRV 下实际执行且非零测试。

## STOP

- 实现要求持久化组级 applied/locked，形成第二份状态真相。
- 前端必须循环单参数命令才能完成批量。
- 同一文件在一个批次需要多次备份或写入。
- 某旧非法状态只能通过静默猜测才能迁移。
