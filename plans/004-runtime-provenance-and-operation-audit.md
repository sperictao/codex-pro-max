# Plan 004：增加多角色运行证据与最小化操作审计

> 执行者：运行时审计只读 Codex 本地产物，绝不修改/复制实时数据库，不返回原始 JSONL
> 或日志文本。冲突证据必须变成 ambiguous，不能选择“最新看起来正确”的一条。

## 状态

- 优先级：P1
- 工作量：L
- 风险：HIGH
- 依赖：Plan 003
- 分支：`codex/guard-groups-agent-roles`

## 证据边界

本地可以分别证明：父线程请求的 `agent_type/fork_turns/override`、子线程
`turn_context.model/effort`、客户端发往 `/responses` 的 requested model。当前没有
服务端 `actual_model/served_model` 字段，因此结果永远不得声称服务端内部实际执行模型。

原子审计单位固定为 `(child_thread_id, turn_id)`。聚合优先级：

`mismatch > ambiguous > unsupported > incomplete > match`。

主要文件：新增 `audit.rs`、`audit_rollout.rs`、`audit_sqlite.rs`、
`operation_audit.rs`，修改 `commands.rs`、`main.rs`、`Cargo.toml`、`Cargo.lock`、生成合同、
清洗 fixtures 与 `.github/workflows/quality.yml`；新增精确 integration targets：
`src-tauri/tests/guard_runtime_audit.rs`、`src-tauri/tests/guard_operation_audit.rs`。

## Step 1：冻结多角色审计 DTO 与隐私合同

新增 `audit.rs` DTO v1：

```rust
struct SubagentAuditResult {
    schema_version: u32,
    checked_at_ms: u64,
    source_support: AuditSourceSupport,
    agents: Vec<AgentAudit>,
    notices: Vec<AuditNotice>,
}

struct AgentAudit {
    thread_id: String,
    role_id: String,
    expected_policy_revision: Option<u64>,
    expected_policy_hash: Option<String>,
    expected_model: Option<String>,
    expected_effort: Option<String>,
    parent: ParentDispatchAudit,
    turns: Vec<TurnAudit>,
    verdict: AuditVerdict,
}
```

- parent match：`agent_type` 精确等于该托管角色、`fork_turns == "none"`、spawn 参数中
  model 与 reasoning_effort 均不存在。
- client match：child `agent_role`、turn model/effort 与该角色在审计开始时的只读 policy
  snapshot 一致，且 `multi_agent_version == "v2"`。
- outbound match：同一 turn 的所有去重 WebSocket sampling 证据模型一致并等于 expected。
- 未托管/已停止管理的 role、角色在审计快照中不存在、Plan 003 的 policy_updated_at 晚于
  child start 时，
  不猜历史期望，标记 incomplete/unsupported 并给稳定 notice。
- DTO 允许 thread/turn UUID、role ID、model/effort、时间、枚举和计数；禁止 agent path、
  task name、message、prompt、arguments 原文、日志 body、数据库路径、完整用户路径和密钥。
- DTO/command 由 Plan 001 合同生成器输出；提交的 Rust 序列化 fixture 由 Rust producer
  逐字比较并由 TypeScript 读取，未知 schemaVersion 不局部渲染为成功。

## Step 2：实现有界 JSONL 流式解析与确定性父子归并

新增 `audit_rollout.rs`，锁定当前观测字段：

- `session_meta.payload.id/parent_thread_id/agent_path/agent_role/source.subagent...`
- `turn_context.payload.turn_id/model/effort/multi_agent_version`
- 父线程 `response_item` 中 `spawn_agent` 的 arguments JSON；只保留 task_name、
  agent_type、fork_turns、model、reasoning_effort，message 不解码。

边界：最近 7 个日期目录、最多 500 rollout、单文件 16 MiB、单行 2 MiB、最多 20 个
child、标识 token 128 ASCII 字符。逐行读取，不把整文件载入内存。

选择顺序固定为日期/文件时间新到旧；触达任何目录、文件或 child 上限都必须增加
`audit_truncated` notice，并让总体结果至少 Incomplete，不能因截断范围内恰好全 match
而显示完整绿色。

归并规则：

1. child 按 parent_thread_id 找唯一 parent rollout。
2. task_name 仅作为内部临时 join key，来自 agent_path 最后一段，永不出 DTO。
3. 同 parent/task_name 的 spawn 与 child start 按时间区间一一配对，最大延迟 120 秒。
4. 零候选为 missing；多候选、相同时间、坏时间或数量冲突均 ambiguous，不做 nearest/latest。
5. 同一 child/turn 的重复 turn_context 完全一致才去重；任一 model/effort/V2 冲突即
   ambiguous。

夹具覆盖重复任务名、多个角色、多 turn、孤儿 child、显式同值 override、错误 role、
错误 fork、冲突 context、超限和加密无关字段。

## Step 3：只读归并两个 SQLite sampling 来源

新增 `audit_sqlite.rs`，同时考虑：

- `~/.codex/logs_2.sqlite`
- `~/.codex/sqlite/logs_2.sqlite`

固定 `rusqlite` 精确版本并启用 bundled；开始实现前先用 Rust 1.89.0 和三平台验证该版本，
失败即 STOP 记录依赖决策，不浮动到“最新兼容”。

- 使用 `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`、`query_only=ON`、100ms busy timeout。
- 先检查 `logs` 表列；参数化查询 thread_id，只读取当前已知 feedback_tags evidence class。
- 单 body 上限 256 KiB；使用确定性 marker scanner 解析
  `try_run_sampling_request{turn_id=... model=...}` 与
  `model_client.stream_responses_websocket{model=...}`，不对任意文本跑宽泛 regex。
- 同一 turn/evidence/model 的重复行跨库去重；不同 model、行内两 marker 不一致或无法
  一一对应都 ambiguous。缺一个库不覆盖另一个库的有效证据，跨库冲突必须暴露。
- 错误、Debug、panic、fixture snapshot 都不得包含原 body。

测试还必须断言数据库调用前后字节一致、忙库/缺表/字段漂移/超限/injection-looking ID
均为安全非绿状态。

## Step 4：提供按需运行的安全命令

内部 `audit_at(codex_root, managed_role_snapshot)` 接受注入路径；生产命令
`guard_run_subagent_audit` 不接受路径，只解析当前用户 Codex 根目录。

- 使用独立 single-flight 读锁；重复调用返回 `audit_already_running`。
- 不阻塞 Guard 写协调器，也不修改 Guard/role/session/database；审计看到配置事务 Busy
  时允许读取已提交 role snapshot，不读取未提交草稿。
- agents 按 started_at 新到旧，turns 按时间旧到新稳定排序。
- 正常的 missing/ambiguous/unsupported 返回结构化结果，不抛含 prose 的异常。
- 角色模型或 effort 在审计期间变更不会改写本次 snapshot。
- `guard_run_subagent_audit`、`guard_operation_audit_list`、DTO 和 single-flight error 加入生成
  合同并在 `src-tauri/src/main.rs:925-964` 注册；command registry test 与最小 IPC smoke
  必须证明前端可调用，不以内部单测代替注册验证。

## Step 5：记录最小化本地操作审计

新增 `operation_audit.rs`，与事务 journal 明确分开：

- 记录时间、batch/作用域 ID、相对文件标识、阶段、结果、错误码、changed/unchanged/files
  计数，以及运行审计观察到的 role/model/effort。
- 不记录角色指令、任务提示、文件内容、完整路径、命令 stdout/stderr、密钥。
- 本地 versioned JSONL 最多保留 500 条或 30 天，先到者清理；清理通过原子替换。
- 事务 journal 建立失败必须阻止配置写；普通 audit append 失败返回/持久显示稳定 error，
  但不反向破坏已提交事务。
- 提供只读 `guard_operation_audit_list`；不在本期提供导出、上传或遥测。

测试：保留边界、时钟边界、坏行、并发 append、脱敏白名单、原型/控制字符输入和 audit
失败不伪装成功。

## Step 6：平台与 MSRV 质量门

扩展 Plan 001 的 `rust-msrv-targeted` matrix，在 macOS/Linux/Windows 上使用最终确认的
MSRV，运行 roles/capability/migration、rollout/sqlite/operation audit 和 command registry
测试；其中 `guard_runtime_audit` 从公开命令边界归并 JSONL + 双 SQLite，
`guard_operation_audit` 验证白名单、保留策略和失败语义。现有 stable Clippy/全量测试保留。
矩阵只使用清洗 fixture，不要求安装真实 Codex。

## 验证

```bash
npm run check:contracts
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_runtime_audit --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --test guard_operation_audit --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::audit_rollout --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::audit_sqlite --timeout 60
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --filter codex_guard::operation_audit --timeout 60
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-run
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --timeout 60
cargo clippy --manifest-path src-tauri/Cargo.toml --locked -- -D warnings
cargo +1.89.0 check --manifest-path src-tauri/Cargo.toml --locked
npm test
git diff --check
```

## Done

JSONL 有界归并、双 SQLite 只读采样、冲突/截断降级、隐私白名单和操作审计存储已落地；
状态保持 IN PROGRESS，操作审计命令覆盖和 append 失败的持久可见性仍需最终核验。

- [ ] 每个 child turn 的 parent/client/outbound 证据确定性归并。
- [ ] 任一冲突或多候选保持 ambiguous，不选 latest/first。
- [ ] 多角色按 role ID 对照各自期望 model/effort。
- [ ] 所有 DTO、日志、错误和审计记录符合最小化白名单。
- [ ] 两个 SQLite 来源只读且冲突可见。
- [ ] 扫描触达任何上限都会产生 truncated notice，结果不能假绿。
- [ ] 审计命令已注册且合同生成/IPC smoke 通过。
- [ ] 本地操作审计按 30 天/500 条清理，无遥测。
- [ ] Rust 1.89.0 与三平台 fixture matrix 全绿。
- [ ] 服务端 actual model 明确保持不可观测。
- [ ] 两个公开边界 integration targets 在三平台与最终 MSRV 下实际执行且非零测试。

## STOP

- 归并必须解密或返回 prompt/message/log body 才能继续。
- SQLite 需要写入、复制或 read-write 打开。
- 内部格式变化无法唯一关联 turn，却准备选择一条候选。
- 依赖无法通过 MSRV/平台门，或错误边界会泄漏原始内容。
