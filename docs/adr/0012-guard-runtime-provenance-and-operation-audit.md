# Guard 运行证据与最小化本地操作审计

## 决策

Guard 把“配置规则是否正确”和“某次子代理运行是否符合规则”分成两个只读层。运行核验
只读取 Codex 本地产物，不修改或复制 rollout、SQLite 或角色文件；它最多证明父线程的
`agent_type`/`fork_turns`、子线程的 model/effort 和客户端请求的 model，不能声称服务端
实际提供的模型，因为当前产物没有 `actual_model`/`served_model` 字段。

审计原子单位是 `(child_thread_id, turn_id)`。父派生、客户端 turn context 和 outbound
sampling evidence 按稳定 ID、时间窗口和角色 ID 归并；零候选、多个候选、冲突值、未知
格式或触达上限都降级为 `missing`、`ambiguous`、`incomplete` 或 `unsupported`，绝不选择
“最新看起来正确”的记录。JSONL 只读取最近的有界日期/文件/行，SQLite 只读打开并设置
`query_only`，同时采样两个已知来源，跨库冲突必须可见。

## 隐私与持久化

返回 DTO、错误、日志和本地操作审计只允许稳定 ID、角色 ID、模型、effort、时间、阶段、
结果、错误码和计数。禁止任务名、agent path、prompt、message、完整 arguments、日志 body、
数据库路径、完整用户路径和密钥。运行审计结果按需执行，不进入普通轮询。

Guard 操作审计与事务 journal 分离：journal 是写入恢复前置条件，审计是已完成操作的最小
摘要。操作审计最多保留 500 条或 30 天，使用原子替换清理；append 失败不回滚已提交配置，
但必须写入稳定的错误记录并通过 `guard_operation_audit_list` 可见。若错误记录本身也无法
持久化，命令日志只能记录稳定 code，绝不回显底层路径或原文。

## 取舍

- 不解析或上传原始 rollout/SQLite 内容，牺牲部分可诊断细节换取本地隐私边界。
- 不把能力快照当运行证明；角色策略在审计开始时冻结，策略更新不能改写历史结论。
- 不在服务端不可观测时猜测 actual model；界面必须明确展示“服务端实际模型不可观测”。
- 审计单飞运行，不阻塞 Guard 写协调器；Guard 事务进行中时只读取上一次已提交角色快照。

## 验收证据

`guard_runtime_audit` 覆盖有界 JSONL、重复/冲突归并、双 SQLite 只读采样、截断降级和隐私
脱敏；`guard_operation_audit` 覆盖白名单、保留策略、坏记录和错误边界。两者都必须在 Rust
1.89.0 的三平台 targeted matrix 中实际执行，不能只依赖模块单测或 Linux stable。
