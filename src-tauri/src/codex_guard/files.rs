//! 看守文件：目标文件列表的持久化与路径检测（列表是看守范围的唯一事实来源）

use std::path::Path;

use crate::config;

use super::GuardFile;

pub(crate) fn builtin_files() -> Vec<GuardFile> {
    vec![
        GuardFile {
            id: "builtin.config-toml".to_string(),
            name: "config.toml".to_string(),
            file: "config.toml".to_string(),
            format: "toml".to_string(),
            builtin: true,
            detection: None,
        },
        GuardFile {
            id: "builtin.agents-md".to_string(),
            name: "AGENTS.md".to_string(),
            file: "AGENTS.md".to_string(),
            format: "md".to_string(),
            builtin: true,
            detection: None,
        },
        GuardFile {
            id: "builtin.default-toml".to_string(),
            name: "default.toml".to_string(),
            file: "agents/default.toml".to_string(),
            format: "toml".to_string(),
            builtin: true,
            detection: None,
        },
    ]
}

/// 加载文件列表；若配置为空则初始化内置文件并持久化
pub(crate) fn load_files() -> Result<Vec<GuardFile>, String> {
    let mut cfg = config::load_config()?;
    if cfg.codex_guard.files.is_empty() {
        cfg.codex_guard.files = builtin_files();
        config::save_config(&cfg)?;
    }
    Ok(cfg.codex_guard.files.clone())
}

pub(crate) fn save_files(files: &[GuardFile]) -> Result<(), String> {
    let mut cfg = config::load_config()?;
    cfg.codex_guard.files = files.to_vec();
    config::save_config(&cfg)
}

pub(crate) fn find_file(files: &[GuardFile], id: &str) -> Option<GuardFile> {
    files.iter().find(|f| f.id == id).cloned()
}

// ponytail: 只搜顶层 + 一层子目录；配置散得更深再升级递归
fn detect_file_path_in(home: &Path, rel: &str) -> Option<String> {
    if home.join(rel).exists() {
        return Some(rel.to_string());
    }
    let name = Path::new(rel).file_name()?.to_string_lossy().to_string();
    for e in std::fs::read_dir(home).ok()?.flatten() {
        let dir = e.path();
        if dir.is_dir() && dir.join(&name).exists() {
            return Some(format!("{}/{}", e.file_name().to_string_lossy(), name));
        }
    }
    None
}

pub(crate) fn detect_file_path(rel: &str) -> Option<String> {
    detect_file_path_in(&crate::codex_fs::codex_home().ok()?, rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::validate::validate_guard_file;

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
        let home = std::env::temp_dir().join(format!("dashi-detect-test-{}", std::process::id()));
        std::fs::create_dir_all(home.join("agents")).unwrap();
        std::fs::write(home.join("config.toml"), "").unwrap();
        std::fs::write(home.join("agents/default.toml"), "").unwrap();

        // 原位置命中
        assert_eq!(detect_file_path_in(&home, "config.toml"), Some("config.toml".into()));
        assert_eq!(
            detect_file_path_in(&home, "agents/default.toml"),
            Some("agents/default.toml".into())
        );
        // 配置写顶层但实际在子目录 → 浅搜命中
        assert_eq!(detect_file_path_in(&home, "default.toml"), Some("agents/default.toml".into()));
        // 不存在 → None
        assert_eq!(detect_file_path_in(&home, "nope.toml"), None);

        std::fs::remove_dir_all(&home).ok();
    }
}
