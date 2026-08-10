# Plan 001：建立 Guard 事务写入与格式校验底座

> 执行者：先写失败测试，再实现；所有文件系统测试必须注入临时路径。任何无法证明的
> 原子替换或恢复语义都应 STOP，不得退化为“尽量写完”。完成后更新计划索引状态。

## 状态

- 状态：IN PROGRESS
- 优先级：P0
- 工作量：L
- 风险：HIGH
- 依赖：无
- 分支：`codex/guard-groups-agent-roles`
- 规划基线：`6235f93`

## 根因与完成后不变量

当前 `engine.rs:108-149` 直接写文件，`config.rs:175-185`、`schema.rs:80-103`
和 `backup.rs:38-47` 都使用截断式 `fs::write`；`poll.rs:20-59` 与 Tauri 命令各自
执行 load-modify-save，没有共享互斥。格式字段只在 `validate.rs:32-45` 校验名字，
`engine` 完全不使用它。

本计划完成后的不变量：

> Guard 的任何写入都必须在同一个 `GuardCoordinator` 内，先完成整作用域预检、事务
> 日志和文件快照，再使用可验证的原子替换；只有写后复查全部成功才提交状态。进程中断
> 后，新写入和轮询必须等待未完成事务恢复。

## 作用域

主要修改：

- `src-tauri/src/codex_guard/mod.rs`
- 新增 `model.rs`、`paths.rs`、`format.rs`、`ownership.rs`
- 新增 `atomic_store.rs`、`transaction.rs`、`journal.rs`
- 改造 `engine.rs`、`backup.rs`、`markdown_block.rs`、`validate.rs`
- 改造 `commands.rs`、`files.rs`、`schema.rs`、`poll.rs`，迁走所有现存直写入口
- 改造 `src-tauri/src/config.rs`、`src-tauri/src/main.rs`、`src-tauri/src/fastctx.rs`
- `src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`
- 新增 `src-tauri/src/codex_guard/contracts.rs`、`src/generated/guard-contracts.ts`
- 新增 `scripts/check-guard-contracts.mjs`、`scripts/run-rust-tests.mjs`
- 事务/格式夹具放入 `src-tauri/src/codex_guard/fixtures/`
- 新增精确 integration targets：`src-tauri/tests/guard_paths_contract.rs`、
  `guard_atomic_store.rs`、`guard_transaction_recovery.rs`

本计划不实现逻辑组、角色 UI、运行时审计。

## Step 0：先修复不可达的 MSRV 基线

当前 `src-tauri/Cargo.toml` 声明 Rust 1.77.2，但锁文件中的可达 `cargo_metadata`、
`serde_with`、`time`、`notify-rust` 已要求更高版本。功能代码开始前必须先执行：

1. 用 `cargo metadata --locked` 和 `cargo tree -i` 固定全部超出 MSRV 的依赖链；
2. 优先在保持当前 Tauri 主版本和安全更新的前提下精确 pin 到 1.77.2 可用闭包；
3. 在三平台运行 `cargo +1.77.2 test --locked --no-run`；
4. 若只能通过降级到不受支持的 Tauri/安全版本才能维持 1.77.2，STOP，单独提交 MSRV
   决策让用户确认提高 `rust-version`，不得让计划继续携带虚假的 1.77.2 门。

这一提交只允许依赖/CI/说明变化，不混入 Guard 功能代码。

### Step 0 实测判定（2026-08-09）

- `cargo +1.77.2 test --locked --no-run` 在解析锁定的 `hashbrown 0.17.1` manifest 时即因
  Edition 2024 不受 Cargo 1.77.2 支持而失败，尚未进入项目编译。
- `cargo metadata --locked` 显示当前可达闭包有 68 个包声明的最低 Rust 高于 1.77.2，
  最高为 `notify-rust 4.18.0` 的 1.89；涉及 Tauri notification、updater、single-instance
  及共享工具链，不是四个传递包的局部 pin。
- `cargo +1.89.0 test --locked --no-run` 在本机 macOS 通过；同一工具链实际执行现有
  31 个 Rust 测试全部通过，60 秒硬超时内完成。
- `git blame` 证明 1.77.2 来自初始化提交；现有 Quality 和发布工作流始终只使用浮动
  stable，从未把 1.77.2 作为 CI 验收门。

因此 Step 0 触发原定 STOP。用户已于 2026-08-09 确认推荐方案：把真实 MSRV 提高到
精确 Rust 1.89.0，在 macOS/Linux/Windows 增加 locked metadata/check/test，并让 release
构建使用同一工具链与 Cargo `--locked`。保留 1.77.2 的整树降级方案不再实施。

## Step 1：先建立可隔离的路径与原子存储测试壳

新增不可序列化到前端的内部类型：

```rust
pub(crate) struct AppPaths {
    home_root: PathBuf,
    launcher_root: PathBuf,
    codex_root: PathBuf,
    transaction_root: PathBuf,
    backup_root: PathBuf,
}
```

- 生产 composition root 是唯一读取 HOME/USERPROFILE 的位置；公开 Tauri command 不接受
  path/root 参数。Config、Guard、FastCtx Guard 边界和 skill 路径改为接收 `AppPaths`。
- 单元和集成测试直接注入 `AppPaths::for_test(tempdir)`，不得修改进程级 `HOME`。
- 把 Launcher `config.json`、磁盘 schema、Guard 文件列表的读写收口到一个
  `ConfigStore`，消除 `load_files()` 读时写默认值的副作用。
- 先写测试证明：损坏配置不会回落默认后继续写；保存失败保留旧字节；两个并发更新不会
  丢字段；测试路径始终位于 tempdir。
- 静态门要求 `std::env::var("HOME"|"USERPROFILE")` 只能存在于 composition root；业务
  模块不得自行重新解析。Node test runner 为每次测试进程创建临时 HOME/USERPROFILE，
  保留真实 CARGO_HOME/RUSTUP_HOME，作为依赖注入之外的外围安全网。

验证：

```bash
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::paths --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter config::tests --timeout 60
```

新增跨平台 `scripts/run-rust-tests.mjs`：先用 `cargo test -- --list` 断言目标前缀至少命中
一个精确测试名，再执行并施加 60 秒硬超时；任何 0 tests 都失败。CI 冷编译使用
`cargo test --no-run --locked`，不把编译时间算入 60 秒测试预算。

### Step 1 实施结果（2026-08-10）

- 已完成 `AppPaths` 注入：生产代码只在 `main.rs` 的 composition root 读取
  `HOME/USERPROFILE`，Config、Guard、FastCtx、skill 与 Node 探测都使用共享路径对象；
  集成测试用源码静态门锁定该约束。
- 已完成 `ConfigStore` 收口：Launcher 配置、Guard schema 与 Guard 文件列表通过同一共享
  互斥更新；损坏文件 fail closed，读取默认文件列表不再写磁盘。
- 已完成单文件原子替换壳：同目录唯一临时文件写入并 flush 后替换目标，保留既有权限并
  拒绝只读目标；Unix 使用 rename，Windows 使用 replace-existing + write-through API。
  跨文件事务、journal 与恢复仍留在 Step 5。
- 已完成跨平台 Rust test runner：list-before-run、0 tests 失败、执行阶段 60 秒超时、临时
  HOME/USERPROFILE 隔离并保留真实 CARGO_HOME/RUSTUP_HOME；Quality 的三平台 MSRV 矩阵已接入。
- 验证通过：Rust 1.89.0 `--locked --no-run`、Clippy `-D warnings`、39 个单元测试与 1 个
  集成测试、runner 自测 3 项、前端测试与生产构建、workflow YAML 解析、`git diff --check`。

Step 1 已完成；Plan 001 继续保持 `IN PROGRESS`。Step 1.5 已完成，Step 2 正在实施。

### Step 1.5 实施结果（2026-08-10）

- 已完成 Guard IPC 的 Rust 单一来源合同：现有 Guard commands、DTO、参数对象和返回值由
  `specta/tauri-specta` 生成到 `src/generated/guard-contracts.ts`；前端 Guard 页面不再直接
  手写 `invoke`。
- 已加入生成合同逐字比较、固定 fixture、command registry 静态门和运行时 decoder；未知
  `schemaVersion` 或枚举值会明确失败。
- 已验证 Rust 1.89.0、Clippy、全量 Rust tests、前端 tests、contracts check 和生产构建。
- 生成器版本兼容性结论已记录：当前锁定的 `specta` 版本可用，未来升级前仍需重新验证
  MSRV/Tauri 兼容性。

## Step 1.5：建立 Rust 单一来源的 Guard IPC 合同

- 使用与最终 MSRV、Tauri 版本通过 Step 0 验证的精确版本 `specta/tauri-specta` 生成
  全部现有及新增 Guard DTO、command 名、参数对象、返回值、event 名和 payload wrapper；
  现存 `guard_add_custom_param` 的 `applyMode/valueType` 与 Rust snake_case 漂移必须先用红测
  复现并由生成 wrapper 消除。
- 生成物提交为 `src/generated/guard-contracts.ts`，业务前端禁止手写 Guard invoke 名和参数。
- `npm run check:contracts` 生成到临时目录并逐字比较，不依赖整个 worktree clean。
- 固定 JSON fixture 放在 `src-tauri/src/codex_guard/fixtures/contracts/`；Rust producer test
  与提交字节比较，TS consumer 读取同一文件。
- 若所选生成器不能支持最终 MSRV/Tauri 或不能覆盖 command/event wrapper，STOP 并记录
  ADR；不得退回当前“两个接口分别手写、分别编译”的假合同。
- 生成合同还必须包含 schemaVersion 与 enum 的运行时允许值；typed wrapper 在返回 DTO
  前执行 envelope/enum decoder，未知版本或枚举返回非绿 contract error，不能只依赖
  `invoke<T>` 的编译期断言。

## Step 2：把文件格式变成类型并实现统一校验管线

在 `model.rs` 定义：

```rust
enum GuardFileFormat { Toml, Json, Markdown, PlainText }

struct ValidationDiagnostic {
    code: DiagnosticCode,
    severity: DiagnosticSeverity,
    scope_id: String,
    relative_file: Option<String>,
    field: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
    params: DiagnosticParams,
}
```

- `GuardFile.format` 改为枚举；反序列化兼容旧值 `md`，迁移后只序列化 `markdown`。
- `format.rs` 负责物理文件格式：
  - TOML：整文件解析，语法/重复键返回行列；
  - JSON：使用自定义 Serde Visitor 拒绝任何对象层级的重复键，不能接受 last-wins；
  - Markdown：扫描全部 `dashi:begin/end` 标记，拒绝重复、交叉、逆序、残缺；
  - PlainText：要求 UTF-8 且无 NUL。
- `validate.rs` 只负责参数/领域语义，不再校验字符串格式名。
- 所有诊断只包含相对路径、稳定 code 和白名单参数，解析器错误不得携带文件片段。
- 删除 `engine.rs` 写路径中的 `unwrap_or_default()`；仅 `NotFound` 可在明确允许新建时形成
  空文档，PermissionDenied 等错误直接失败。

测试矩阵：四格式 valid/invalid、JSON 多层重复键、Markdown 重复/交叉/残缺、非 UTF-8、
NUL、错误行列、超大诊断脱敏。

### Step 2 实施结果（2026-08-10）

- 已新增 `GuardFileFormat` 和结构化 `ValidationDiagnostic`；旧配置中的 `md` 仍可读取，
  保存时统一写成 `markdown`，并补齐 `json`、`markdown`、`plain_text` 的 canonical 合同值。
- 已新增统一 bytes 校验 seam：TOML 全文件解析、JSON 递归拒绝重复键、Markdown 标记结构
  校验、PlainText UTF-8/NUL 校验；诊断只返回稳定 code、相对路径和行列，不回显源文或解析器
  错误文本。
- 已移除 Guard 写路径中把读取错误静默当空文件的行为；仅明确允许新建的 TOML/Markdown
  路径使用空文档，权限错误、目录目标和坏格式均 fail closed。
- 已将文件格式枚举接入文件列表、视图、轮询、命令和前端选择器，并增加格式/模式兼容校验。
- Step 4 的多成员写计划以及 Step 5 的事务日志仍未开始。

### Step 3 基础实施结果（2026-08-10）

- 已新增 `ownership.rs` 作为 Guard 文件、参数和 TOML 路径的统一预检入口：相对路径规范化、根目录 containment、最近存在父目录解析，以及目标/中间路径的 symlink 拒绝。
- 已接入 schema 合并、文件增删改、启用、应用、锁定、视图和轮询入口；重复文件 ID/路径、父子路径、未知文件引用、重复参数 ID、TOML parent/child、`toml_absent` 冲突和 `file_overwrite` 排他都会在写入前失败。
- FastCtx 保留键仍由 `fastctx.rs` 单一列出，并按 TOML segment 边界拒绝 Guard 托管；相似前缀不会误伤。
- 本步只完成所有权预检，不宣称完成快照后/替换前的 no-follow 目录句柄、TOCTOU 防护和崩溃恢复；这些仍由 Step 5 的事务写入层负责。

## Step 3：建立唯一配置所有权检查

`ownership.rs` 在任何写计划生成前校验：

- 文件路径解析为规范化真实路径；目标不存在时规范化最近存在父目录并追加剩余组件。
- Windows 比较大小写不敏感且统一分隔符；拒绝 symlink/别名导致的重复所有权。
- `GuardFile.id`、规范化路径、参数 ID全局唯一；逻辑组 ID 与 file→group 隔离在 Plan 002
  模型落地后再校验，本计划只返回物理文件 blocker set。
- 同一 TOML/JSON 路径不得重复，也不得出现 parent/child、`toml_absent`/子路径冲突。
- `file_overwrite` 对物理文件排他；后续角色文件同样使用该排他规则。
- AGENTS.md 以托管区块为所有权边界，区块名不得重复。
- fastctx 所有键继续由 `fastctx.rs` 明确列入禁止清单；Guard 保存前就拒绝冲突。
- snapshot 前和每次 replace 前都重新验证 canonical identity、根目录 containment 与父目录
  身份；使用 no-follow/目录句柄语义避免预检后被 symlink 调包。仅复核内容 hash 不够。

先写 table-driven 冲突测试，再接入 schema 保存、文件新增/编辑与写入预检。冲突不得等到
轮询时才暴露。

## Step 4：把 engine 改成纯计划器

将当前 `engine::apply` 拆成无 I/O 的计划阶段：

```rust
fn plan_file_write(
    file: &ManagedFile,
    members: &[ManagedMember],
    original: &[u8],
) -> Result<PlannedFileWrite, Vec<ValidationDiagnostic>>;
```

- 同一文件一次解析，按稳定路径顺序合并全部成员，生成一份候选字节。
- 候选字节再次通过格式校验；计划同时保存原 SHA-256、候选 SHA-256 和 post-check 描述。
- `check` 与 `render` 共享同一解析/路径实现，避免写与读两套语义。
- 现有单参数命令暂时可以构造单成员计划，但不得继续调用直写 `engine::apply`。
- 删除或私有化任何可绕过事务协调器的写函数。

测试证明：同文件八个参数只产生一个最终文档；输入顺序不改变输出；未知 apply mode、
类型错误、读取错误都在生成写计划前失败。

## Step 5：实现事务日志、快照和跨平台原子替换

`transaction.rs` 定义阶段：

`Preflight → Snapshot → Writing → PostCheck → Restoring → Completed`

`journal.rs` 使用版本化 envelope，至少记录批次 ID、阶段、相对目标、原/候选 hash、
快照引用和完成位图，不记录文件内容。具体文件字节放在权限受限的批次快照目录。

写入协议：

1. 全作用域预检完成后，为每个物理文件生成唯一批次快照；事务快照位于 journal 批次
   目录，只保留到提交/成功恢复，Critical 时保留。另建立用户可用 durable backup，按
   规范化文件目录 + batch UUID 命名并继续每文件保留 20 份。
2. snapshot 与 journal 均 `sync_all`，journal 成功持久化后才允许首次目标写入。
3. 每个目标落盘前重新读取并核对原 hash；发现外部抢写即停止并恢复此前已写文件。
4. 临时文件必须与目标同目录，写入后 flush/sync，再执行平台原子替换并同步父目录。
5. `config.json` 作为事务参与者：journal 记录其前像 hash、快照、状态提交位图和 commit
   marker；目标文件与 Launcher lifecycle 状态一起提交/恢复。
6. 全部写完后逐文件 post-check，再原子写状态并落 commit marker；任一失败恢复目标和
   Launcher 状态全批次。
7. 恢复成功后记录 rolled_back 报告；恢复失败保留 journal/快照并进入 Critical Error。

先做一个被测试锁定的平台适配器：Unix 使用同目录 rename；Windows 使用经 MSRV 和
三平台测试证明的 replace-existing API。若 `std`/既有依赖无法证明 Windows 语义，新增
精确版本的 `windows-sys` 适配；不得以“Windows 以后再说”完成本计划。

为 I/O 注入故障点：journal create/sync、snapshot N、pre-write identity/hash、write N、
replace、post-check N、state-save 前后、commit-marker 前后、restore N、journal cleanup。
每个故障点都断言目标字节、Launcher lifecycle、journal、Critical 状态和报告一致。

除可恢复 Err 测试外，`guard_transaction_recovery` 必须做真实双进程 crash 测试：子进程在
journal durable、snapshot durable、write-N、post-check 四类屏障暂停；父测试在 Unix 发送
SIGKILL、Windows 调用 TerminateProcess，再启动第二进程走生产 recovery。逐字节断言目标、
Launcher 状态、journal/快照、Critical 和 poll-blocking。没有该三平台测试，Plan 001 不得
DONE。

## Step 6：接管所有 Guard 及相邻写入口

- 在 `main.rs:19-22,858-903` 的 Tauri state 中加入 `GuardCoordinator` 和共享
  `ConfigStore`；启动顺序改为“加载/迁移前置检查 → 恢复未完成事务 → 启动 poll”。
- 协调器只提供非排队 `try_lock`：用户命令忙时返回 `guard_busy`；poll 忙时跳过本轮；
  禁止积压过期意图。
- `update_settings`、公开 `save_config`、Guard schema/files mutation 必须通过共享
  `ConfigStore`，避免旧整份 LauncherConfig 覆盖新状态。
- Launcher 主动执行 fastctx apply/unapply 前获取同一 Guard 写锁；忙则拒绝。外部程序
  自行修改仍由 pre-write hash/漂移机制发现。
- recovery API 先提供后端命令 `guard_get_recovery_status`、
  `guard_retry_recovery`；计划 005 再实现 UI。
- Critical Error 在本计划只封锁涉及的物理文件集合；Plan 002 再从 file→group/全局派生
  scope blocker，避免在逻辑组模型存在前建立反向依赖。
- 新 Guard command/transaction/FastCtx lock 错误统一返回稳定 error envelope 与白名单参数；
  Guard 边界不得透传完整路径或 CLI stdout/stderr。既有 FastCtx 结果展示合同不在本期
  重构，但获取 Guard 锁失败必须使用新安全 code。

## 验证

```bash
npm run check:contracts
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_paths_contract --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_atomic_store --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_transaction_recovery --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::format --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::ownership --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::transaction --timeout 60
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-run
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --timeout 60
cargo clippy --manifest-path src-tauri/Cargo.toml --locked -- -D warnings
cargo +1.89.0 check --manifest-path src-tauri/Cargo.toml --locked
git diff --check
```

CI 新增独立 `rust-msrv-targeted` matrix：recursive submodule checkout；仅 Linux 安装
GTK/WebKit；三平台可移植创建 `vendor/dashi-taskboard/dist/web`；固定最终 MSRV，先
`--no-run --locked`，再用 Node runner 跑 paths/contracts/atomic/transaction。stable
Ubuntu 全测/Clippy job 保持独立。所有新增直接依赖必须固定并通过该 matrix。

## Done

- [ ] 四种格式在任何写前和写后使用同一校验实现。
- [ ] 配置/schema/目标文件损坏不再回落默认后继续写。
- [ ] 同一文件每批次只快照、备份、原子写、post-check 一次。
- [ ] 任一阶段失败不会留下业务层部分成功；恢复失败进入可重试 Critical Error。
- [ ] 应用重启先处理未完成事务，再启动轮询。
- [ ] 三平台双进程强杀测试证明 durable journal 的真实崩溃恢复，而非只测可返回的 I/O Err。
- [ ] 目标文件与 Launcher lifecycle 状态共同提交/恢复，commit marker 故障不产生错配。
- [ ] durable backup 每文件保留 20 份，事务快照按完成/Critical 规则清理。
- [ ] 所有 Guard、LauncherConfig 和 FastCtx 主动写入口遵守协调器边界。
- [ ] Guard IPC 类型、命令、参数和 event 由 Rust 生成并通过合同漂移门。
- [ ] 自动化测试只使用注入临时目录，三平台与 MSRV 全绿。
- [ ] 静态路径门证明业务模块不直接读取 HOME/USERPROFILE，测试进程使用 disposable home。

## STOP

- 无法在某支持平台证明 replace-existing 与 crash recovery 语义。
- 需要把任意根目录暴露给生产 Tauri command 才能测试。
- 某写入口必须保留协调器外直写，导致并发不变量无法成立。
- 错误处理需要记录配置原文、指令或密钥。
