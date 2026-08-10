//! 看守文件：目标文件列表的持久化与路径检测（列表是看守范围的唯一事实来源）

use std::path::Path;

use crate::config::ConfigStore;

use super::{AppPaths, GuardFile, GuardFileFormat};

pub(crate) fn builtin_files() -> Vec<GuardFile> {
    vec![
        GuardFile {
            id: "builtin.config-toml".to_string(),
            name: "config.toml".to_string(),
            file: "config.toml".to_string(),
            format: GuardFileFormat::Toml,
            builtin: true,
            detection: None,
        },
        GuardFile {
            id: "builtin.agents-md".to_string(),
            name: "AGENTS.md".to_string(),
            file: "AGENTS.md".to_string(),
            format: GuardFileFormat::Markdown,
            builtin: true,
            detection: None,
        },
        GuardFile {
            id: "builtin.default-toml".to_string(),
            name: "default.toml".to_string(),
            file: "agents/default.toml".to_string(),
            format: GuardFileFormat::Toml,
            builtin: true,
            detection: None,
        },
    ]
}

/// 加载有效文件列表；空持久状态只在内存投影内置文件，不产生写副作用。
pub(crate) fn load_files(store: &ConfigStore) -> Result<Vec<GuardFile>, String> {
    let cfg = store.load_launcher()?;
    Ok(if cfg.codex_guard.files.is_empty() {
        builtin_files()
    } else {
        cfg.codex_guard.files
    })
}

pub(crate) fn update_files<R>(
    store: &ConfigStore,
    update: impl FnOnce(&mut Vec<GuardFile>) -> Result<R, String>,
) -> Result<R, String> {
    store.update_launcher(|config| {
        let mut files = if config.codex_guard.files.is_empty() {
            builtin_files()
        } else {
            std::mem::take(&mut config.codex_guard.files)
        };
        let result = update(&mut files)?;
        config.codex_guard.files = files;
        Ok(result)
    })
}

pub(crate) fn find_file(files: &[GuardFile], id: &str) -> Option<GuardFile> {
    files.iter().find(|f| f.id == id).cloned()
}

// ponytail: 只搜顶层 + 一层子目录；配置散得更深再升级递归
fn detect_file_path_in(home: &Path, rel: &str) -> Option<String> {
    if is_regular_non_symlink(&home.join(rel)) {
        return Some(rel.to_string());
    }
    let name = Path::new(rel).file_name()?.to_string_lossy().to_string();
    for e in std::fs::read_dir(home).ok()?.flatten() {
        let dir = e.path();
        let is_dir = std::fs::symlink_metadata(&dir)
            .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            .unwrap_or(false);
        if is_dir && is_regular_non_symlink(&dir.join(&name)) {
            return Some(format!("{}/{}", e.file_name().to_string_lossy(), name));
        }
    }
    None
}

fn is_regular_non_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

pub(crate) fn detect_file_path(paths: &AppPaths, rel: &str) -> Option<String> {
    detect_file_path_in(paths.codex_root(), rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::validate::validate_guard_file;
    use crate::config::ConfigStore;

    #[test]
    fn builtin_files_has_three_entries() {
        let files = builtin_files();
        assert_eq!(files.len(), 3);
        for f in &files {
            assert!(f.builtin, "{} 应该是内置文件", f.id);
            assert!(f.id.starts_with("builtin."));
            validate_guard_file(f).unwrap();
        }
    }

    #[test]
    fn detect_file_path_finds_config_and_shallow_nested() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        std::fs::create_dir_all(home.join("agents")).unwrap();
        std::fs::write(home.join("config.toml"), "").unwrap();
        std::fs::write(home.join("agents/default.toml"), "").unwrap();

        // 原位置命中
        assert_eq!(
            detect_file_path_in(&home, "config.toml"),
            Some("config.toml".into())
        );
        assert_eq!(
            detect_file_path_in(&home, "agents/default.toml"),
            Some("agents/default.toml".into())
        );
        // 配置写顶层但实际在子目录 → 浅搜命中
        assert_eq!(
            detect_file_path_in(&home, "default.toml"),
            Some("agents/default.toml".into())
        );
        // 不存在 → None
        assert_eq!(detect_file_path_in(&home, "nope.toml"), None);
    }

    #[cfg(unix)]
    #[test]
    fn detect_file_path_does_not_follow_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        std::fs::create_dir_all(home.join("agents")).unwrap();
        let outside = temp.path().join("outside.toml");
        std::fs::write(&outside, "x = 1\n").unwrap();
        std::os::unix::fs::symlink(&outside, home.join("config.toml")).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("agents/default.toml")).unwrap();

        assert_eq!(detect_file_path_in(&home, "config.toml"), None);
        assert_eq!(detect_file_path_in(&home, "default.toml"), None);
    }

    #[test]
    fn load_files_projects_builtins_without_writing_config() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let store = ConfigStore::new(paths.clone());

        let files = load_files(&store).unwrap();

        assert_eq!(files.len(), 3);
        assert!(!paths.config_file().exists());
    }
}
