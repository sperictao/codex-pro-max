# Guard IPC 合同由 Rust 单一来源生成

Guard 的 DTO、命令参数与返回类型采用 `tauri-specta 2.0.0-rc.21` 配合 `specta 2.0.0-rc.22` 生成 TypeScript 合同，并固定提交生成物供前端消费；这两个精确版本已在 Rust 1.89.0 与当前 Tauri 2.11.5 上验证可编译。更新的 rc.24/rc.25 依赖 Rust 1.89 尚未稳定的 `fmt::from_fn`，因此暂不采用；生成器升级必须重新通过 MSRV 验证并更新本 ADR。运行时解码器负责 schema 版本和允许枚举校验，不能把 `invoke<T>` 的编译期类型断言当作协议校验。
