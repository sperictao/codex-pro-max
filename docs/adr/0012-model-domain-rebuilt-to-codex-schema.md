# 模型域按 Codex 自身 schema 重建（不自建供应商模型，未知键透传）

模型域此前只管三键加 `[model_providers.<id>]` 的四个字段（`name` / `base_url` / `env_key` / `experimental_bearer_token`）。本次把 dsh-pro-max（同作者的另一款启动器，管理 DeepSeek Harness 的模型域）的模型配置工作台整体移植过来：供应商段扩展到 Codex 的全部托管键，另加 models.dev 目录、环境变量就绪状态、连通性验证与本机导入。可移植的是**能力与交互**，不是它的配置 schema。

移植的前提是一个判断：**存储与语义一律以 Codex 自身的 `config.toml` 为准，不引入第二份模型真相**。

**Considered Options**：

- **照搬 dsh 的 `llm-pi-ai.providers` 形态（自定义 providers.json / settings 段）** —— 拒绝。Codex 不读这个概念：它的供应商就是 `config.toml` 的 `[model_providers.<id>]` 表，协议是 `wire_api = chat|responses`（不是 dsh 的 `openai-completions|openai-responses|anthropic-messages`），模型条目是请求时传的字符串（没有 dsh 的 `models[]` 数组声明）。落一份并行配置等于让启动器写 Codex 不读的键，违反 Clean 的第一条。
- **保留旧的四字段投影，只把 UI 换成新皮** —— 拒绝。真实 `~/.codex/config.toml` 里已经在用 `wire_api` / `requires_openai_auth` / `supports_websockets`，官方参考表另有 `query_params` / `http_headers` / `env_http_headers` / `request_max_retries` / `stream_max_retries` / `stream_idle_timeout_ms`。只认四个字段的编辑器会把这些键**看不见地留着**，用户改完不知道哪些生效；而"看不见"与"被删掉"在体验上同样糟。
- **把托管键写成固定白名单并丢弃其余键** —— 拒绝。Codex 的供应商表在演进（`requires_openai_auth` / `supports_websockets` 就是参考表没列但已在用的键）。丢弃未知键等于让启动器成为配置的破坏者。
- **把未知键读进 `extra` 但不写回** —— 拒绝。读得进写不回，用户一保存就丢配置；半吊子透传比不透传更危险。
- **用 models.dev 目录作为模型事实来源（判断模型是否合法、拒绝未列出的 id）** —— 拒绝。本地自建端点（Ollama / vLLM / 中转）的模型按定义不在目录里，用它做准入会拦下完全合法的配置。目录只回答"官方标称多少容量/支持哪些档"，不回答"能不能用"。
- **让前端直接抓 models.dev** —— 拒绝。目录是 27MB 级 JSON，webview 受 CSP `connect-src` 限制且要过 CORS；放 Rust 侧还能顺带做按路由裁剪，只把相关条目送进 IPC。
- **把 `experimental_bearer_token` 明文回传到前端再原样写回** —— 拒绝。密钥来回穿一趟 IPC 只为了在输入框里显示掩码，属于无收益的暴露面；改为只上行不下行。
- **凭据只认进程环境** —— 拒绝。Codex 只认环境变量名，用户修不了父进程环境；把 `~/.codex/.env` 纳入可写层，界面才有一条"能修好"的路径（并如实标注哪些层是只读的）。

**Consequences**：

- 后端拆成五个模块，各管一个关注点，落盘路径只有 `model_manifest` 一处：`model_schema`（托管键投影 + `extra` 透传 + 校验）、`model_catalog`（models.dev 快照与按路由投影）、`model_credentials`（进程环境 → 用户 `.env` → 项目 `.env` 的分层解析）、`model_remote`（连通性验证与远端模型列表）、`model_import`（四个来源的扫描与导入）；`model_config` 只做编排与命令面。
- **未文档化的键透传**：托管键之外的键进 `ProviderSchema.extra`，保存时按"编辑前读到的键名单"（`previous_unknown_keys`）先清后写——用户在 UI 里删掉的透传键会真的消失，没动过的原样保留。手写的子表（`[model_providers.x.y]`）不在删除范围。
- **就地改写而非整条替换**：toml_edit 把"键前一行的注释"存成键值对的 decor，`table.insert` 整条替换会连注释一起丢。托管键因此走 `put_value`/`put_item`（类型相容时只换 value），用户注释与排版得以保留。
- **密钥只写不读**：`ProviderSchema.bearer_token` 标了 `skip_serializing`，任何视图与调试输出都带不出真值；存在性由 `has_bearer_token` 表达。写回三态：`None` = 不动磁盘原值（编辑时输入框留空）、`Some("")` = 显式清除、`Some(v)` = 写入。
- **档位表放宽**：官方参考列 `minimal|low|medium|high`，但真实配置里已在用 `max` 与 `none`。取并集（`none|minimal|low|medium|high|xhigh|max`），列表偏窄会让应用层拒绝用户手上真实存在的值。档位表只有 Rust 一处事实来源，前端经视图的 `efforts` 取用。
- **目录是缓存不是事实来源**：快照落在 `~/.codex-pro-max/models-cache.json`，过期只提示不阻塞；缓存损坏按"没有目录"降级，绝不让模型页报错。目录不覆盖某路由时状态是 `unlisted`（不告警），覆盖但查不到 id 时才是 `unknown`（提示可能是拼写错误）。
- **导入只搬环境变量引用**：来源持明文密钥的一律不读取、不展示、不落盘，只计数提示；已存在的路由由后端跳过，不覆盖用户现有配置。
- 前端新增 `features/models/` 下的 `ModelsView` / `ProviderDialog` / `ModelPickerDialog` / `ImportDialog` / `HeadersEditor` / `ops`，全部走 [ADR 0011](0011-shell-component-primitive-layer.md) 的组件原语层；纯推导（校验、归一化、目录判定、格式化）集中在 `ops.ts` 单测。
- 新增 Rust 依赖 `reqwest`（`rustls-tls` + `json` + `gzip`，异步客户端）：目录约 27MB，需要 gzip 传输与免系统 openssl 的 TLS；联网命令是 async（跑在 tauri 的 tokio 运行时上），不阻塞同步 IPC 通道。
- 复选框一律用 `Switch` 原语：craft-spec 明确"全仓 checkbox 均作开关用、0 真实调用点"，本域新增的三处布尔字段照此办理。
- 测试基础设施补了 jsdom 缺失的 Pointer Events 捕获方法与 `scrollIntoView`（`src/shared/test/setup.ts`）：Radix 的 Select / DropdownMenu 在 `pointerdown` 时会调用前者，缺失即菜单永远打不开。
