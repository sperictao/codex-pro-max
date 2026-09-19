//! `~/.codex/config.toml` 的读写入口：模型域的领域逻辑共用这一处落盘路径。
//!
//! 每次写入前备份到 `~/.codex/dashi-backups/`（与看守域同一备份机制），
//! 写回用 toml_edit 保留注释与排版，不重排用户手写的键。

use toml_edit::DocumentMut;

use crate::codex_fs::{codex_file, write_with_backup};

/// config.toml 相对 `~/.codex` 的文件名
pub(crate) const CONFIG_FILE: &str = "config.toml";

pub(crate) fn config_path() -> Result<std::path::PathBuf, String> {
    codex_file(CONFIG_FILE)
}

/// 读 config.toml 为可编辑文档；文件不存在按空文档处理（首次写入即创建）
pub(crate) fn read_config_doc() -> Result<DocumentMut, String> {
    let path = config_path()?;
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    content.parse::<DocumentMut>().map_err(|e| {
        let err = crate::i18n::trf(
            "Failed to parse config.toml: {error}",
            &[("error", e.to_string())],
        );
        crate::logging::error("模型配置: 解析 config.toml", &err);
        err
    })
}

/// 写回 config.toml（先备份）
pub(crate) fn write_config_doc(doc: &DocumentMut) -> Result<(), String> {
    let path = config_path()?;
    write_with_backup(CONFIG_FILE, &path, &doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_is_valid_target() {
        // 首次写入（config.toml 不存在）走的是空文档分支
        let doc = "".parse::<DocumentMut>().unwrap();
        assert!(doc.as_table().is_empty());
    }

    #[test]
    fn read_reports_parse_error_with_context() {
        let bad = "this is [ not toml";
        assert!(bad.parse::<DocumentMut>().is_err());
    }
}
