use std::path::{Path, PathBuf};

/// 进程内唯一的用户目录派生来源。
///
/// 该类型只由 composition root 使用已解析的主目录构造，不读取环境变量，
/// 也不创建目录；测试可以安全注入临时根目录。
#[derive(Debug, Clone)]
pub(crate) struct AppPaths {
    home_root: PathBuf,
    launcher_root: PathBuf,
    codex_root: PathBuf,
    #[allow(dead_code)] // Plan 001 Step 5 consumes the predeclared journal root.
    transaction_root: PathBuf,
    backup_root: PathBuf,
}

impl AppPaths {
    pub(crate) fn from_home(home_root: PathBuf) -> Self {
        let launcher_root = home_root.join(".dashi-taskboard-launcher");
        let codex_root = home_root.join(".codex");
        Self {
            transaction_root: launcher_root.join("transactions"),
            backup_root: codex_root.join("dashi-backups"),
            home_root,
            launcher_root,
            codex_root,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(root: &Path) -> Self {
        Self::from_home(root.join("home"))
    }

    pub(crate) fn home_root(&self) -> &Path {
        &self.home_root
    }

    #[allow(dead_code)] // Kept as part of the path contract for later store participants.
    pub(crate) fn launcher_root(&self) -> &Path {
        &self.launcher_root
    }

    pub(crate) fn codex_root(&self) -> &Path {
        &self.codex_root
    }

    #[allow(dead_code)] // Plan 001 Step 5 consumes the predeclared journal root.
    pub(crate) fn transaction_root(&self) -> &Path {
        &self.transaction_root
    }

    pub(crate) fn backup_root(&self) -> &Path {
        &self.backup_root
    }

    pub(crate) fn config_file(&self) -> PathBuf {
        self.launcher_root.join("config.json")
    }

    pub(crate) fn guard_schema_file(&self) -> PathBuf {
        self.launcher_root.join("codex-guard-schema.json")
    }

    pub(crate) fn codex_file(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.codex_root.join(relative)
    }

    pub(crate) fn codex_skills_root(&self) -> PathBuf {
        self.codex_root.join("skills")
    }

    pub(crate) fn manage_taskboard_skill_dir(&self) -> PathBuf {
        self.codex_skills_root().join("manage-taskboard")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_test_paths_stay_under_tempdir_without_creating_directories() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let derived = [
            paths.home_root().to_path_buf(),
            paths.launcher_root().to_path_buf(),
            paths.codex_root().to_path_buf(),
            paths.transaction_root().to_path_buf(),
            paths.backup_root().to_path_buf(),
            paths.config_file(),
            paths.guard_schema_file(),
            paths.codex_file("agents/default.toml"),
            paths.manage_taskboard_skill_dir(),
        ];

        for path in derived {
            assert!(
                path.starts_with(temp.path()),
                "path escaped tempdir: {path:?}"
            );
            assert!(
                !path.exists(),
                "path constructor created filesystem state: {path:?}"
            );
        }
    }
}
