# Plan 003：支持多个结构化子代理角色

> 执行者：只在 Plan 002 完成后开始。角色文件是模型与推理强度的配置源；AGENTS.md
> 只负责选角，不得复制完整指令或形成第二份模型配置。现有磁盘角色只发现，不自动接管。

## 状态

- 优先级：P0
- 工作量：L
- 风险：HIGH
- 依赖：Plan 002
- 分支：`codex/guard-groups-agent-roles`

## 根因与不变量

当前 `guard_schema.json:134-145` 只把 `agents/default.toml` 当本地化整文件字符串，无法
结构化校验，也无法安全管理多个角色。磁盘上虽可新增任意 GuardFile，但没有角色 ID、
模型能力、文件所有权、发现/纳入、default 保护或迁移规则。

完成后：

> 一个托管角色对应一个稳定 Role ID、一份排他拥有的 `agents/<id>.toml` 和一组结构化
> 字段；多个角色同时存在。角色文件声明模型/effort，AGENTS 角色目录要求主代理用精确
> `agent_type`、`fork_turns="none"` 且省略单次 model/effort 覆盖。

主要文件：新增 `roles.rs`、`role_directory.rs`、`capability.rs`，扩展 `migration.rs`、
`model.rs`、`commands.rs`、`view.rs`、`schema.rs`、`guard_schema.json`、`main.rs`、生成合同
与角色/capability fixtures；新增精确 integration targets：
`src-tauri/tests/guard_roles_capability.rs`、`src-tauri/tests/guard_role_migration.rs`、
`src-tauri/tests/guard_command_registry.rs`。

## Step 1：定义角色模型、边界和持久化

新增 `roles.rs`，定义：

```rust
struct ManagedRoleRecord {
    id: RoleId,
    selection_criteria: String,
    order: u32,
    expected_toml: String,
    policy_revision: u64,
    policy_hash: String,
    policy_updated_at_ms: u64,
    description_origin: TextOrigin,
    instructions_origin: TextOrigin,
}
```

- `RoleId` 同时用于 `agent_type` 和文件名主体，正则
  `[a-z][a-z0-9-]{0,62}`，大小写不敏感全局唯一，创建后不可修改。
- `default` 是保留 ID，固定排序第一，可编辑内容但不可复制覆盖、停止管理、重命名或删除。
- 显示名称 1–80 字符、用途和选择条件各 1–500 字符、指令 1–64 KiB；文本为 UTF-8、
  trim 后非空、拒绝 NUL。
- 最多托管 32 个角色；接近上限的 warning 阈值定为 28，33 个时硬拒绝。
- 所有角色固定归入内置 `subagent-optimization` 组。
- `expected_toml` 是角色 TOML 的唯一持久事实源；display name、purpose、model、effort、
  instructions 均由它解析成 `ManagedRoleView`，不得同时持久化重复字段。保存结构化输入时
  用 `toml_edit::DocumentMut` 更新该文档，保留注释/未知字段；UI 原文只读。
- 每次成功保存增加 `policy_revision`，重算只包含已知策略字段的 hash 与更新时间，供 Plan
  004 判断运行记录是否早于当前策略；格式性注释变化不改变 policy hash。
- 内置 default 的 description/instructions 保留现有中英文产品默认和用户覆盖 origin：未被
  用户修改时随界面语言解析，用户修改后固定为用户值；model/effort 永远语言无关。自定义
  角色只有用户文本，不自动翻译。
- 新角色使用最小模板且生命周期为 Disabled；保存只持久化 Guard 期望状态，不创建实际
  角色文件，首次 Apply/Lock 才经 Plan 002 批次创建。

角色文件已知键固定为 `name`、`description`、`model`、
`model_reasoning_effort`、`developer_instructions`；选择条件只属于 Launcher/角色目录，
不擅自写成 Codex 未定义的 TOML 键。

测试：ID 边界、大小写碰撞、路径穿越、各字段长度、NUL、0/1/28/32/33 角色、default
保护、模板更新保留注释和未知字段。

## Step 2：发现、诊断和显式纳入已有角色

- 只扫描 Codex 根目录直属 `agents/*.toml`；拒绝越出 `agents/` 的 symlink 目标。
- 扫描结果分为 managed/discovered，按大小写不敏感 ID 去重；全部未托管角色都显示，
  不受 32 个托管上限裁剪。
- discovered summary 只返回 ID、相对文件名、语法/能力诊断和是否可纳入，不返回原文或
  指令。详情按需读取且仍只返回结构化字段和只读脱敏原文视图。
- 语法无效角色只能诊断，不可纳入；语法有效未知字段/注释保留并给 Warning。
- 当前 Codex 正式诊断明确拒绝未知字段时升级为 Invalid；Guard 自己不猜测未知字段含义。
- `guard_role_adopt(id)` 必须再次校验实时文件 hash、解析、所有权、角色上限和能力，随后
  把当前文档作为 expected_toml；不写实际角色文件。
- 磁盘已有 ID、Codex 当前已知角色和新建 ID 冲突时，新建拒绝；纳入必须走 adopt，不借
  “新建”覆盖。

测试包含无效 TOML、未知字段、symlink 越界、大小写同名、扫描中途文件变化、纳入前
hash 改变和全部 discovered 可见。

## Step 3：迁移旧 `agents-default-toml`

在 `migration.rs` 增加独立的角色迁移计划：

1. 读取旧 Guard expected（含持久 `state.value`）与实际 `agents/default.toml`，但先不写。
2. 两者均有效且语义、完整文档一致时，自动解析为结构化 `default`，保留原生命周期、
   时间戳、注释和未知字段。
3. 任一无效或两者冲突时进入 Pending，迁移向导只提供：
   - `adopt_live`：采用当前实际文件；
   - `adopt_stored`：采用旧 Guard expected；
   - `use_safe_default`：建立产品安全模板。
4. 用户选择前不写 `default.toml`；选择保存后才移除旧 whole-file owner。任何时刻禁止
   旧 owner 与结构化角色同时锁定同一文件。
5. 其他 `agents/*.toml` 只进入 discovered，不自动纳入。

同一版本迁移必须升级会破坏多角色的旧内置 schema：

- `features.multi_agent_v2.hide_spawn_agent_metadata` 的产品默认改为 false，使 agent_type
  可观测；新增/确认 `expose_spawn_agent_model_overrides=false` 作为配置硬化，但不把它当
  运行证明；
- 保留并实时验证 `multi_agent_v2.enabled=true`、`tool_namespace="agents"`；
- 退休“全部派生只能 default”的旧 AGENTS 托管段，替换为 Step 5 角色目录；
- 退休硬编码 Luna 的 whole-file 模板。新安装和 `use_safe_default` 必须实时读取模型目录，
  让用户确认目录标记的 V2 默认模型及其 default effort；没有可用组合时保持 Pending，
  不写入一个可能已过时的硬编码模型。

如果旧生命周期是 Locked，但选中的迁移来源与实际文件不同，迁移结果降为 Disabled 并
要求用户显式 Apply/Lock；不得在迁移过程中自动覆盖角色文件。迁移幂等，保存中断后可
根据旧备份和阶段安全重试。

## Step 4：实现当前 Codex 能力探测和模型选择合同

新增 `capability.rs`：

- 从已配置的 Codex Desktop 安装位置解析打包的 `codex` 可执行文件，不从任意 PATH 或
  第三方 launcher 猜测。
- 无 shell 执行 `codex doctor --json` 和 `codex debug models`；每个命令 10 秒硬超时，
  stdout/stderr 各限 2 MiB，不向 DTO 返回原输出。
- 只接受机器字段：配置加载、角色警告、`models[].slug`、
  `supported_reasoning_levels[].effort`、`multi_agent_version`。
- 模型选择器只列当前目录中 `multi_agent_version == "v2"` 的模型；effort 按模型动态过滤；
  不允许自由文本模型。
- capability snapshot 保存模型/effort 枚举、Codex 可执行文件身份和时间，不保存命令输出。
- 离线时 snapshot 只允许展示和保留 UI 内存草稿，不得提交 `ManagedRoleRecord`；角色 save、
  Apply、Lock 都必须实时刷新成功。读取失败标记 Error，不冒充 Unsupported；只有实时目录
  明确移除组合才标记 Unsupported。
- capability change 不自动改角色、角色目录或目标文件。Unsupported 角色阻塞其所在组及
  全局 Apply/Lock；Unlock、Disable、Stop Managing、Delete 仍可执行。
- 平台 resolver 覆盖项目当前支持的 macOS app、Windows packaged identity 和 Linux 安装
  形态；找不到可执行文件、启动失败或布局未知均返回 Error。只有已成功读取的模型目录
  明确缺少 model/effort/V2 时才返回 Unsupported；禁止回退另一套 Codex CLI。

测试使用注入式可执行程序/fixture，覆盖合法、malformed role、模型缺失、effort 缺失、
V1、超时、超大输出、非零退出、字段漂移、缓存失效和无 PATH fallback。新增依赖必须
通过 Rust 1.89.0 与三平台。

## Step 5：生成唯一的 AGENTS 角色目录

在 `roles.rs` 或独立 `role_directory.rs` 生成
`<!-- dashi:begin/end subagent-role-directory -->` 区块：

- 按 default 第一、其余用户顺序列出已托管且实际角色文件存在的角色。
- 每项只包含经 Markdown 转义并压成单行的 ID、显示名称、用途、选择条件。
- 区块统一声明：使用 `agent_type=<id>`、必须 `fork_turns="none"`、省略 spawn 的
  `model`/`reasoning_effort`；角色文件是模型/effort 配置源。
- 不复制完整 developer instructions，不生成任意 Markdown/HTML，不覆盖 AGENTS.md 其他
  内容或其他托管区块。
- 新建但从未 Apply 的角色不进入目录；已 Apply 后再 Disable 因实际文件仍保留，角色仍
  留在目录。Stop Managing/Delete 才原子移除目录项。
- 目录与角色文件参加同一个 `subagent-optimization` 组批次；同一批次每文件仍只写一次。

目录只能证明配置规则，不能证明某次派生遵守；运行证据由 Plan 004 提供。

## Step 6：实现角色命令与原子退出流程

新增命令：

- `guard_role_get(id)`、`guard_role_save(input)`、`guard_role_copy(source, input)`
- `guard_role_discover()`、`guard_role_adopt(id)`、`guard_role_reorder(ids)`
- `guard_role_stop_managing(id)`、`guard_role_delete(id)`
- `guard_capability_get()`、`guard_capability_refresh()`
- `guard_role_migration_resolve(choice)`
- 角色卡 Apply/Lock/Unlock/Disable 调用 Plan 002 的
  `guard_execute_batch(BatchScope::Role { role_id })`，不新增第二套角色生命周期命令。

规则：

- UI 草稿不持久化；save 前执行实时完整角色校验。copy 只复制用途、模型、effort 和指令，
  选择条件留空，要求新 ID/名称，并从最小模板开始；不复制未知字段、注释、生命周期、
  健康、revision 或审计。
- Stop Managing 是单一事务：解锁/禁用、移出角色目录和 Guard state，保留角色文件，
  成功后重新进入 discovered。
- Delete 非默认角色同样原子执行：备份角色文件，移除文件、目录项和 Guard state；失败
  恢复全部。确认交互在 Plan 005，但后端必须返回文件数和备份引用摘要。
- 角色排序不影响 ID 或文件路径；批次运行时拒绝排序/保存。
- 全部命令使用 Coordinator 非排队锁和稳定错误 code。
- 全部角色/能力 DTO、command 和 Role-scope event 进入生成合同，并在
  `src-tauri/src/main.rs:925-964` 逐项注册；增加 command registry 测试和最小 IPC smoke，
  防止 Rust 单测全绿但前端 invoke 未注册。

## 验证

```bash
npm run check:contracts
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_roles_capability --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_role_migration --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_command_registry --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::roles --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::capability --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::migration --timeout 60
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-run
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --timeout 60
cargo clippy --manifest-path src-tauri/Cargo.toml --locked -- -D warnings
cargo +1.89.0 check --manifest-path src-tauri/Cargo.toml --locked
npm test
npm run build
git diff --check
```

Plan 001 的 `rust-msrv-targeted` matrix 在本提交扩展为三平台实际执行上述三个 integration
targets；它们分别覆盖角色/能力边界、default 迁移和 command 注册。仅 `cargo check`、模块
过滤器或 Ubuntu stable 不足以完成本计划。

## Done

结构化角色、发现/纳入、复制、排序、停止管理、删除、能力探测、AGENTS 目录和 default
迁移已落地并有角色/能力/迁移/注册目标测试；状态保持 IN PROGRESS，等待最终跨平台矩阵和
隔离桌面验收证据。

- [ ] 1–32 个角色可同时托管，ID/文件名/agent_type 恒等。
- [ ] 新建、复制、发现、纳入、排序、停止管理、删除均符合原子与 default 保护规则。
- [ ] 结构化保存保留未知字段/注释，原文没有第二编辑入口。
- [ ] 模型/effort 只来自当前 V2 能力目录；离线不授权 Apply/Lock。
- [ ] 离线 capability 只保留 UI 草稿，不持久化新的角色期望。
- [ ] 旧 default 三分支迁移不自动覆盖实际文件，不产生双 owner。
- [ ] 旧 default-only AGENTS、metadata 和 Luna 模板已迁移，不会继续压制多角色。
- [ ] policy revision/hash/time 足以让运行审计判断策略时间关系。
- [ ] AGENTS 角色目录最小化、转义、无完整指令，并固定 fresh-context 派生合同。
- [ ] 三个公开边界 integration targets 在三平台与最终 MSRV 下实际执行且非零测试。

## STOP

- Codex 当前机器字段无法证明模型、effort 或 V2 支持，却准备显示 Supported。
- 角色模板保存会丢未知字段/注释或要求开放原始编辑。
- 迁移冲突需要静默选择来源。
- 角色退出必须绕过事务协调器或留下陈旧目录项。
