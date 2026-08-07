# FastCtx 集成委托 CLI，启动器不自写 TOML

设置页「集成」区的 fastctx 开关（见 CONTEXT.md "FastCtx 集成"域），接入/摘除一律调 fastctx 自己的 CLI（`fastctx apply --yes` / `fastctx unapply --yes`），启动器不复用 codex_guard 的 TOML 写入机制去改 `~/.codex/config.toml`。

反直觉之处：启动器明明有一整套 schema 驱动的 TOML 写入/比对机制，却不用。原因是 fastctx 写哪些键、写什么值（`mcp_servers.fastctx` 表、`features.code_mode.direct_only_tool_namespaces` 数组、共享键 `tool_output_token_limit` 的档位映射、legacy namespace 迁移）是它自己的实现细节且持续演进，启动器复刻一份必然腐烂；且 apply 还附带启动器写不了的步骤（二进制固化到 `~/.fastctx/bin/`、ChatGPT 桌面端配置、共享键冲突处理与回执）。委托 CLI 后这些全部免费，启动器只保留：安装检测、config.toml 接入状态读取、`status` 自检、未安装引导。

**Considered Options**：自写 TOML（开关 = 启动器直接插/删 `mcp_servers.fastctx`）——不依赖 CLI 在场、摘除可以只摘配置保留数据，但要追踪 fastctx 的配置演进、跳过二进制固化，且半摘除残留（code_mode 数组等）是长期坑；否决。**Consequences**：CLI 不在 PATH 时开关不可用，UI 只能引导用户 `npm i -g fastctx`；"关"等价于 unapply 的重操作（杀进程、删 `~/.fastctx` 受管数据），靠 npm 全局包保留保证可重新接入。
