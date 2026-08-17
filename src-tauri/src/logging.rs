//! 统一错误日志：所有返回前端（Result<T, String>，i18n 文案）的错误路径
//! 在此落盘一次，避免各处裸 `return Err(trf(...))` 再现「用户看到、日志没有」。
//! 日志只进文件不进 UI（CONTEXT.md 边界）；级别约定：读/探测类失败用 warn，
//! 写盘/进程事故/用户明确操作失败用 error。

/// 记录一条 error 日志并原样返回错误字符串（供 `.map_err(logging::fail)` 链式落盘）。
/// 用法：`config::save_config(&cfg).map_err(|e| logging::fail("保存看守配置", &e))?;`
pub fn fail(context: &str, err: &str) -> String {
    error(context, err);
    err.to_string()
}

/// 落一条 error 日志。
pub fn error(context: &str, err: &str) {
    log::error!("[{}] {}", context, err);
}

/// 落一条 warn 日志。
pub fn warn(context: &str, err: &str) {
    log::warn!("[{}] {}", context, err);
}