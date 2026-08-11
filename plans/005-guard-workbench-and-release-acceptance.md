# Plan 005：交付 Guard 工作台、迁移交互和完整验收

> 执行者：保持 vanilla TypeScript + ES modules。前端只展示后端权威 DTO 和驱动单次
> command，不解析 TOML/JSONL/SQLite，也不重建生命周期、健康或动作 eligibility。

## 状态

- 优先级：P0
- 工作量：L
- 风险：MED
- 依赖：Plan 001–004
- 分支：`codex/guard-groups-agent-roles`

主要文件：`index.html`、`src/guard.ts`、新增 `src/guard-view-model.ts`、生成的
`src/generated/guard-contracts.ts`、`src/shell.ts`、`src/style.css`、双语 i18n、`scripts/`、
`package.json`、Quality workflow、CONTEXT/design/ADR/双语 README。

## Step 1：冻结前端 DTO 与纯 UI reducer

新增 `src/guard-view-model.ts`，无 DOM、Tauri、i18n 运行时依赖：

- 从 `src/generated/guard-contracts.ts` 导入 `GuardView`、BatchPreview/Report、
  RecoveryStatus、Managed/DiscoveredRoleSummary、CapabilitySnapshot、SubagentAuditResult 和
  typed command/event wrappers；禁止 mirror 手写 interface、invoke 名或参数键；
- `reduceGuardUiState` 分开保留最后成功 view、view/files 错误、当前 batch、最终报告、
  Critical Recovery、角色展开状态和最后审计结果；
- `actionOrder = apply → lock → unlock → disable`；
- `shouldConfirmBatch` 只对全局 Apply/Lock 返回 true；组级四动作及全局 Unlock/Disable false；
- tone 映射未知值永不为绿色；诊断参数只复制后端定义的白名单；
- 陈旧 batch ID 事件忽略，普通三秒刷新不得清空报告、恢复状态、展开状态或审计结果。

动作 enabled/reason、生命周期聚合和健康聚合直接使用后端，不在 TS 重算。

新增 `scripts/test-guard-view-model.mjs`，沿用 `scripts/test-theme.mjs` 的临时编译 +
`node:test` 模式，覆盖枚举、确认矩阵、view/error 保留、batch/recovery、陈旧事件、角色
折叠、诊断白名单、prototype-chain 输入和 Rust fixture DTO。

## Step 2：重构 Guard 页面信息架构

在 `index.html:403-420` 的 Guard 工作台加入：

- `#guard-global-summary`：运行状态、全局生命周期、全局健康；
- `#guard-global-actions`：全部启用、锁定、解锁、禁用；
- `#guard-diagnostics`：字段/卡片/页面三级诊断汇总；
- `#guard-operation-region`：六阶段进度和最终 changed/unchanged/files；
- `#guard-recovery-region`：Critical Error、未恢复文件相对标识、重试恢复/打开备份目录；
- `#guard-groups`：只渲染逻辑参数组；
- `#guard-agent-roles` 下的 managed/discovered 纵向角色区。

`src/guard.ts` 改成分区渲染，不再因输入聚焦冻结事务进度；Guard-local 瞬时状态不进入
`src/state.ts`。`renderGuardToggle` 不再在暂停轮询时隐藏 Guard Tab，确保迁移、解锁、
禁用和恢复仍可访问。

全局和组级按钮固定顺序与图标：

1. CircleCheck 启用
2. LockKeyhole 锁定
3. UnlockKeyhole 解锁
4. CircleOff 禁用

不使用播放三角形。桌面显示图标+文字；窄屏仅图标但保留 tooltip、作用域化 aria-label
和可见焦点。禁用动作末尾留视觉间隔；按钮不可执行时保留位置并显示 reason。

## Step 3：实现逻辑组与批量交互

- 所有批量按钮只调用一次 `guard_execute_batch`，busy 时禁用全部写动作并设置
  `aria-busy=true`。
- 全局 Apply/Lock 先调用 `guard_preview_batch`，确认框显示后端返回的成员数与文件数，再
  携 preview ID 执行；stale preview 要求重新确认。全局 Unlock/Disable、组级动作不确认。
- 成功显示 committed 摘要；preflight rejected 持久显示诊断；rolled back 明确写“已恢复”；
  CriticalRecovery 使用 `role="alert"`，绝不显示普通失败或绿色。
- 组卡头常显双状态轴和四动作；正文显示成员、文件引用和字段诊断。内置组在前，自定义
  组可排序，空 Uncategorized 隐藏。
- 组创建/改名/删除/移动参数遵守后端 eligibility；非空删除错误不能只 toast。
- 设置页 GuardFile 新增/编辑下拉必须完整支持 TOML、JSON、Markdown、Plain text；
  PendingFormatMigration 显示显式选择对话框。非法旧 lifecycle 显示独立向导，可选择
  Disabled 或实时 Apply，不能与角色来源迁移混为一谈。
- `shell.ts:261-274,334-387` 继续使用 `data-action` 委托；新增 progress event 在
  `setupEventListener` 接入。

## Step 4：实现多角色卡片、编辑和迁移向导

- managed roles 纵向可折叠，default 固定第一；卡头显示名称/ID/model/effort、生命周期、
  健康和 Role-scope 四动作，正文按需调用 `guard_role_get`，避免三秒 summary 携带
  64 KiB 指令。
- discovered 单独分区，显示语法/能力诊断和“纳入看守”；无效角色没有纳入按钮。
- 新建/复制表单包含 ID、显示名称、用途、选择条件、model、effort、instructions；model
  只用能力目录 select，effort 动态过滤。原始 TOML 只读，未知字段 warning 可见。
- 无效输入或 capability 离线编辑只保留在页面内存草稿，不调用 save；实时能力刷新和完整
  校验成功后才更新 Guard 期望状态。
- Stop Managing 不确认；Delete 角色确认框明确文件和备份，default 不显示两动作。
- 角色迁移向导明确展示 adopt live / adopt stored / safe default，选择前 Guard 写入和轮询
  暂停，其他应用功能可用。对话框必须有 `role=dialog`、`aria-modal`、标题关联、初始焦点、
  Escape 和焦点返回。
- Capability 离线显示 snapshot 时间和 Error；Apply/Lock reason 明确要求实时刷新；
  Unlock/Disable/Stop/Delete 不被错误禁用。

## Step 5：展示运行证据和最小审计

- 提供显式“运行证据核验”，不放进三秒/六十秒轮询。
- 每个 agent 按 role ID 展示 parent dispatch、client model/effort/V2、outbound requested
  model；mismatch destructive，ambiguous/unsupported/incomplete warning，只有完整 match
  为成功。
- 固定提示“服务端实际模型不可观测”；不显示 task name、agent path、源文件、数据库、
  原始 arguments/log body。
- 普通刷新和语言切换保留上次审计；重跑失败保留旧结果并显示新错误。
- 本地操作历史只显示时间、scope、阶段、结果、code 和计数；不增加导出或上传。

## Step 6：完成响应式、无障碍与双语

- 在 `src/style.css:329-408` 增加 action bar、state axes、group/role card、diagnostic、
  transaction/recovery 样式；双轴始终显示轴名和文本，不能只靠颜色。
- 全局/组/恢复动作永久可见；参数动作在窄屏或 `pointer: coarse` 常显。
- 四动作可换行；角色/恢复 dialog 窄屏改单列并滚动；诊断路径/code 可复制，覆盖 body 的
  `select-none`。
- transaction phase 使用 `role=status aria-live=polite`，Critical 使用 `role=alert`。
- label 与 input 用 `for/id`；角色折叠用 `aria-expanded/aria-controls`。
- 新增所有 en/zh-CN key，后端 code 映射为本地化文案；未知 code 显示通用警告但不回显
  未知值。切换语言同步 `<html lang>`。
- 在 200% 缩放、窄窗口、键盘-only、触控指针、亮暗主题下手工验收。

## Step 7：把自动化门禁接入 CI

- `package.json` 的 `npm test` 串接 theme、guard-view-model、assurance 和已提交 Rust DTO
  fixture consumer；新增 `check:contracts` 调用 Plan 001 的临时重生成比较。
- `.github/workflows/quality.yml` 前端 job 依次运行 `npm run check:contracts`、
  `npx tsc --noEmit`、`npm test`、`npm run build`。
- stable Rust job先 `cargo test --locked --no-run`，再用跨平台 Node runner 施加 60 秒执行
  超时，随后 Clippy；另保留 Plan 001/003/004 的三平台 MSRV targeted matrix。
- 不引入 React/Vue/Svelte/jsdom；实际 DOM、焦点和 Tauri 事件在真实桌面冒烟验证。

## Step 8：文档、升级门禁与真实桌面冒烟

文档：

- 更新 `docs/design.md` 的 Guard 模块、命令、数据流和启动恢复顺序；
- `CONTEXT.md`、ADR 0010 与实现逐条对照，必要时仅修正实现命名偏差；
- 新增 ADR 0011，记录多角色配置/能力/dispatch/client/outbound/server 六层保证与隐私边界；
- 更新 README 和 README.zh-CN 的用户操作、迁移、备份、限制和“配置变更只影响新任务”；
- 不把机器专属路径写成跨平台保证。

升级：

- 第一次读取 v0 配置前建立升级备份；迁移 Pending 时 Guard 页面直达向导，其余功能正常。
- 迁移失败不重置；错误说明如何打开备份。旧版本不承诺读取 v1 schema，降级前恢复备份。
- 启动 recovery 优先于 poll；未完成事务必须在 UI 可见。

自动门禁：

```bash
npm run check:contracts
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-run
node scripts/run-rust-tests.mjs --manifest-path src-tauri/Cargo.toml --timeout 60
cargo clippy --manifest-path src-tauri/Cargo.toml --locked -- -D warnings
cargo +1.89.0 check --manifest-path src-tauri/Cargo.toml --locked
git diff --check
npm run tauri -- build --debug --bundles app --no-sign
```

真实桌面冒烟使用 disposable HOME，保留真实 RUSTUP/CARGO 路径并直接启动 source-built
debug app executable。启动前分别用 `127.0.0.1:0` 分配空闲 Taskboard/CDP 端口并写入隔离
Launcher config；不得使用默认 47823/9231、复用已有 Taskboard、调用真实 FastCtx/npm 或
打开真实 Codex。记录真实 Launcher config、`~/.codex` 目标文件 hash 和固定端口监听基线。
覆盖：

1. v0 自动迁移、三种冲突选择、失败后其他功能可用；
2. 四种格式错误定位及零写入；
3. 跨三文件组的四动作、确认矩阵、同文件一次备份/写入；
4. busy 拒绝、外部抢写、post-check 失败、恢复成功；
5. 注入未完成 journal 后重启自动恢复，以及恢复失败 Critical UI/重试；
6. 创建/复制/adopt/stop/delete 多角色，default 保护、32/33 边界、未知字段保留；
7. 模型目录离线、V1/Unsupported、快照可编辑但 Apply/Lock 被阻止；
8. 角色目录内容、fresh-context 合同和无完整指令；
9. 清洗 fixture 的 match/mismatch/ambiguous/incomplete 运行审计；
10. 键盘、焦点返回、200% 缩放、窄屏、中英文、亮暗主题；
11. 审计/日志/UI 不出现指令、提示、原文、密钥或完整用户路径；
12. 关闭应用后检查 disposable HOME 内事务、备份、审计保留和目标文件字节；所有测试
    PID 消失，真实文件 hash 与 47823/9231 监听状态和开始前完全一致。

冒烟证据审阅完成前保留临时目录；清理时移到废纸篓，不对未确认路径执行递归删除。
本次隔离承诺只覆盖 LauncherConfig、Codex 文件、固定端口和测试创建进程；Tauri 自身的
日志、窗口状态等平台 app data 若需零副作用，应在独立 macOS 测试账号执行，不作虚假保证。

## 最终 Done

工作台 UI、六阶段进度、四动作、角色管理、恢复/审计展示、i18n、ARIA 和 CI 门已实现；
本计划仍保持 IN PROGRESS，直到文档和隔离 debug `.app` 的逐项桌面冒烟证据归档。

- [ ] 用户可清楚区分 Guard 运行状态、生命周期、健康和事务 Busy/Recovery。
- [ ] 全局/组级四动作、分组、多角色、迁移和恢复均可从 UI 完成。
- [ ] 所有危险或未知状态非绿色，错误持久可定位且不自动修复。
- [ ] 按钮、对话框、角色折叠和诊断满足键盘/ARIA/窄屏验收。
- [ ] 运行证据层级分明，服务端 actual 保持不可观测。
- [ ] 自动测试、三平台/MSRV、debug build 和逐项真实桌面冒烟全部通过。
- [ ] Guard IPC 合同重生成无 diff，command 注册和最小 IPC smoke 全绿。
- [ ] README、CONTEXT、design、ADR 与实现一致。
- [ ] 未引入本期排除项，也未触碰用户真实 Codex 配置。

## STOP

- 前端需要重建后端状态机或循环旧命令才能完成操作。
- 迁移/恢复错误只能通过 toast 表达，或暂停 Guard 时页面无法进入。
- UI/日志/DTO 需要显示任意原文、角色指令、提示或完整路径。
- 桌面冒烟无法证明运行在 disposable HOME；先修复隔离再测试。
