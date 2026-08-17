//! 备份：任何写入前把目标文件当前内容复制到 ~/.codex/dashi-backups/，每文件保留 20 份

use std::path::{Path, PathBuf};

use crate::i18n::trf;

use super::{codex_home, now_secs};

/// 写入前备份目标文件到 ~/.codex/dashi-backups/，每个文件保留 20 份
fn backup(rel_file: &str, target: &Path) -> Result<(), String> {
    if !target.exists() {
        return Ok(());
    }
    let dir = codex_home()?.join("dashi-backups");
    std::fs::create_dir_all(&dir).map_err(|e| {
        crate::logging::error("看守备份: 创建备份目录", &e.to_string());
        trf("Failed to create backup directory: {error}", &[("error", e.to_string())])
    })?;
    let flat = rel_file.replace(['/', '\\'], "_");
    let dest = dir.join(format!("{}.{}.bak", flat, now_secs()));
    std::fs::copy(target, &dest).map_err(|e| {
        crate::logging::error("看守备份: 写备份文件", &e.to_string());
        trf("Backup failed: {error}", &[("error", e.to_string())])
    })?;

    // 只保留 20 份：文件名即时间戳，字典序可排
    let mut olds: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| {
            crate::logging::error("看守备份: 扫描备份目录", &e.to_string());
            trf("Failed to read backup directory: {error}", &[("error", e.to_string())])
        })?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{}.", flat)) && n.ends_with(".bak"))
        })
        .collect();
    olds.sort();
    while olds.len() > 20 {
        let _ = std::fs::remove_file(olds.remove(0));
    }
    Ok(())
}

pub(crate) fn write_with_backup(rel_file: &str, target: &Path, content: &str) -> Result<(), String> {
    backup(rel_file, target)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::logging::error("看守写入: 创建目录", &e.to_string());
            trf("Failed to create directory: {error}", &[("error", e.to_string())])
        })?;
    }
    std::fs::write(target, content).map_err(|e| {
        crate::logging::error("看守写入", &e.to_string());
        trf("Failed to write {path}: {error}", &[
            ("path", target.display().to_string()),
            ("error", e.to_string()),
        ])
    })
}
