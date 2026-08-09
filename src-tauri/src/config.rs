use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::codex_guard::atomic_store::{AtomicFileWriter, PlatformAtomicFileWriter};
use crate::codex_guard::{AppPaths, GuardParam};

/// 启动器配置，持久化到 ~/.dashi-taskboard-launcher/config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherConfig {
    /// dashi-taskboard 仓库的本地路径
    #[serde(default)]
    pub taskboard_path: String,

    /// Node.js 可执行文件路径（留空则使用 PATH 中的 node）
    #[serde(default)]
    pub node_path: String,

    /// Codex 桌面应用路径
    #[serde(default = "default_codex_path")]
    pub codex_app_path: String,

    /// Taskboard HTTP 服务端口
    #[serde(default = "default_taskboard_port")]
    pub taskboard_port: u16,

    /// Taskboard HTTP 绑定地址
    #[serde(default = "default_taskboard_host")]
    pub taskboard_host: String,

    /// CDP 远程调试端口
    #[serde(default = "default_cdp_port")]
    pub cdp_port: u16,

    /// 是否在启动时自动打开浏览器
    #[serde(default = "default_true")]
    pub auto_open: bool,

    /// 是否使用独立窗口模式（true）或重启模式（false）
    #[serde(default)]
    pub separate_window_mode: bool,

    /// 关闭窗口时是否最小化到系统托盘（false 则退出应用）
    #[serde(default)]
    pub minimize_to_tray_on_close: bool,

    /// 界面语言："system"（跟随系统）/ "en" / "zh-CN"
    #[serde(default = "default_language")]
    pub language: String,

    /// Codex 配置看守状态（见 codex_guard.rs）
    #[serde(default)]
    pub codex_guard: crate::codex_guard::CodexGuardState,
}

fn default_codex_path() -> String {
    #[cfg(target_os = "macos")]
    {
        "/Applications/ChatGPT.app".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        // 不预填猜测路径，由 detect_codex_app 探测真实安装位置
        String::new()
    }
    #[cfg(target_os = "linux")]
    {
        "/usr/bin/chatgpt".to_string()
    }
}

fn default_taskboard_port() -> u16 {
    47823
}

fn default_taskboard_host() -> String {
    "127.0.0.1".to_string()
}

fn default_cdp_port() -> u16 {
    9231
}

fn default_true() -> bool {
    true
}

fn default_language() -> String {
    "system".to_string()
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            taskboard_path: String::new(),
            node_path: String::new(),
            codex_app_path: default_codex_path(),
            taskboard_port: default_taskboard_port(),
            taskboard_host: default_taskboard_host(),
            cdp_port: default_cdp_port(),
            auto_open: true,
            separate_window_mode: false,
            minimize_to_tray_on_close: false,
            language: default_language(),
            codex_guard: Default::default(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ConfigStore {
    paths: AppPaths,
    lock: Arc<Mutex<()>>,
    writer: Arc<dyn AtomicFileWriter>,
}

impl ConfigStore {
    pub(crate) fn new(paths: AppPaths) -> Self {
        Self {
            paths,
            lock: Arc::new(Mutex::new(())),
            writer: Arc::new(PlatformAtomicFileWriter),
        }
    }

    #[cfg(test)]
    fn with_writer(paths: AppPaths, writer: Arc<dyn AtomicFileWriter>) -> Self {
        Self {
            paths,
            lock: Arc::new(Mutex::new(())),
            writer,
        }
    }

    pub(crate) fn load_launcher(&self) -> Result<LauncherConfig, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        self.load_launcher_unlocked()
    }

    pub(crate) fn update_launcher<R>(
        &self,
        update: impl FnOnce(&mut LauncherConfig) -> Result<R, String>,
    ) -> Result<R, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        let mut config = self.load_launcher_unlocked()?;
        let result = update(&mut config)?;
        let bytes = serde_json::to_vec_pretty(&config)
            .map_err(|error| crate::i18n::trf("Failed to serialize config: {error}", &[("error", error.to_string())]))?;
        self.writer.replace(&self.paths.config_file(), &bytes)?;
        Ok(result)
    }

    pub(crate) fn load_guard_schema(&self) -> Result<Vec<GuardParam>, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        self.load_guard_schema_unlocked()
    }

    pub(crate) fn update_guard_schema<R>(
        &self,
        update: impl FnOnce(&mut Vec<GuardParam>) -> Result<R, String>,
    ) -> Result<R, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        let mut schema = self.load_guard_schema_unlocked()?;
        let result = update(&mut schema)?;
        let bytes = serde_json::to_vec_pretty(&schema)
            .map_err(|error| crate::i18n::trf("Failed to serialize schema: {error}", &[("error", error.to_string())]))?;
        self.writer.replace(&self.paths.guard_schema_file(), &bytes)?;
        Ok(result)
    }

    pub(crate) fn ensure_guard_schema_file(&self, default_bytes: &[u8]) -> Result<(), String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        match std::fs::metadata(self.paths.guard_schema_file()) {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.writer.replace(&self.paths.guard_schema_file(), default_bytes)
            }
            Err(error) => Err(crate::i18n::trf(
                "Failed to read schema file: {error}",
                &[("error", error.to_string())],
            )),
        }
    }

    fn load_launcher_unlocked(&self) -> Result<LauncherConfig, String> {
        let path = self.paths.config_file();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LauncherConfig::default());
            }
            Err(error) => {
                return Err(crate::i18n::trf(
                    "Failed to read config file: {error}",
                    &[("error", error.to_string())],
                ));
            }
        };
        let mut config: LauncherConfig = serde_json::from_slice(&bytes).map_err(|error| {
            crate::i18n::trf(
                "Failed to parse config file: {error}",
                &[("error", error.to_string())],
            )
        })?;
        config.taskboard_path = strip_unc(&config.taskboard_path);
        Ok(config)
    }

    fn load_guard_schema_unlocked(&self) -> Result<Vec<GuardParam>, String> {
        let path = self.paths.guard_schema_file();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(crate::i18n::trf(
                    "Failed to read schema file: {error}",
                    &[("error", error.to_string())],
                ));
            }
        };
        serde_json::from_slice(&bytes).map_err(|error| {
            crate::i18n::trf(
                "Failed to parse schema file: {error}",
                &[("error", error.to_string())],
            )
        })
    }
}

/// 剥掉 Windows `\\?\` 扩展路径前缀。
/// Tauri resource_dir 内部 canonicalize 的副作用；CreateProcess 的工作目录参数
/// 不认这个前缀，Node 拿到也别扭，统一剥成普通路径
pub fn strip_unc(s: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{}", rest);
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    s.to_string()
}

/// 用新设置字段更新现有配置，保留 codex_guard 等非设置页字段不变
/// 防止设置页保存时把内存中过时的看守状态写回
pub fn merge_settings(current: &mut LauncherConfig, settings: &LauncherConfig) {
    current.taskboard_path = settings.taskboard_path.clone();
    current.node_path = settings.node_path.clone();
    current.codex_app_path = settings.codex_app_path.clone();
    current.taskboard_port = settings.taskboard_port;
    current.taskboard_host = settings.taskboard_host.clone();
    current.cdp_port = settings.cdp_port;
    current.auto_open = settings.auto_open;
    current.separate_window_mode = settings.separate_window_mode;
    current.minimize_to_tray_on_close = settings.minimize_to_tray_on_close;
    current.language = settings.language.clone();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::{AppPaths, CodexGuardState};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Barrier;

    struct FailingWriter;

    impl AtomicFileWriter for FailingWriter {
        fn replace(&self, _target: &std::path::Path, _bytes: &[u8]) -> Result<(), String> {
            Err("injected replace failure".to_string())
        }
    }

    fn sample_guard_state() -> CodexGuardState {
        use crate::codex_guard::GuardParamState;
        let mut params = HashMap::new();
        params.insert(
            "features.image_generation".to_string(),
            GuardParamState {
                value: Some(serde_json::json!(false)),
                applied: true,
                locked: true,
                last_checked: Some(12345),
                last_restored: Some(12340),
            },
        );
        CodexGuardState {
            enabled: true,
            params,
            files: Vec::new(),
        }
    }

    #[test]
    fn merge_settings_preserves_codex_guard() {
        let mut current = LauncherConfig::default();
        current.codex_guard = sample_guard_state();
        current.taskboard_path = "/old/path".to_string();
        current.auto_open = true;

        let settings = LauncherConfig {
            taskboard_path: "/new/path".to_string(),
            auto_open: false,
            cdp_port: 9999,
            ..Default::default()
        };

        merge_settings(&mut current, &settings);

        // 设置类字段被更新
        assert_eq!(current.taskboard_path, "/new/path");
        assert_eq!(current.auto_open, false);
        assert_eq!(current.cdp_port, 9999);

        // codex_guard 完整保留，没有被默认值覆盖
        assert_eq!(current.codex_guard.enabled, true);
        let p = current.codex_guard.params.get("features.image_generation").unwrap();
        assert_eq!(p.applied, true);
        assert_eq!(p.locked, true);
        assert_eq!(p.last_checked, Some(12345));
        assert_eq!(p.value, Some(serde_json::json!(false)));
    }

    #[test]
    fn merge_settings_updates_all_setting_fields() {
        let mut current = LauncherConfig::default();
        current.codex_guard = sample_guard_state();

        let settings = LauncherConfig {
            taskboard_path: "tp".to_string(),
            node_path: "np".to_string(),
            codex_app_path: "cap".to_string(),
            taskboard_port: 1111,
            taskboard_host: "0.0.0.0".to_string(),
            cdp_port: 2222,
            auto_open: false,
            separate_window_mode: true,
            minimize_to_tray_on_close: true,
            language: "zh-CN".to_string(),
            ..Default::default()
        };

        merge_settings(&mut current, &settings);

        assert_eq!(current.taskboard_path, "tp");
        assert_eq!(current.node_path, "np");
        assert_eq!(current.codex_app_path, "cap");
        assert_eq!(current.taskboard_port, 1111);
        assert_eq!(current.taskboard_host, "0.0.0.0");
        assert_eq!(current.cdp_port, 2222);
        assert_eq!(current.auto_open, false);
        assert_eq!(current.separate_window_mode, true);
        assert_eq!(current.minimize_to_tray_on_close, true);
        assert_eq!(current.language, "zh-CN");

        // codex_guard 不变
        assert_eq!(current.codex_guard.enabled, true);
        assert_eq!(current.codex_guard.params.len(), 1);
    }

    #[test]
    fn default_guard_state_is_disabled_and_empty() {
        let cfg = LauncherConfig::default();
        assert_eq!(cfg.codex_guard.enabled, false);
        assert!(cfg.codex_guard.params.is_empty());
    }

    #[test]
    fn corrupt_launcher_config_blocks_update_without_changing_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.launcher_root()).unwrap();
        let original = b"{ not valid json";
        std::fs::write(paths.config_file(), original).unwrap();
        let store = ConfigStore::new(paths.clone());
        let closure_ran = AtomicBool::new(false);

        let result = store.update_launcher(|config| {
            closure_ran.store(true, Ordering::SeqCst);
            config.language = "en".to_string();
            Ok(())
        });

        assert!(result.is_err());
        assert!(!closure_ran.load(Ordering::SeqCst));
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), original);
    }

    #[test]
    fn corrupt_guard_schema_blocks_update_without_changing_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.launcher_root()).unwrap();
        let original = b"[ not valid json";
        std::fs::write(paths.guard_schema_file(), original).unwrap();
        let store = ConfigStore::new(paths.clone());
        let closure_ran = AtomicBool::new(false);

        let result = store.update_guard_schema(|schema| {
            closure_ran.store(true, Ordering::SeqCst);
            schema.clear();
            Ok(())
        });

        assert!(result.is_err());
        assert!(!closure_ran.load(Ordering::SeqCst));
        assert_eq!(std::fs::read(paths.guard_schema_file()).unwrap(), original);
    }

    #[test]
    fn failed_replace_preserves_old_launcher_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.launcher_root()).unwrap();
        let mut old_config = LauncherConfig::default();
        old_config.language = "zh-CN".to_string();
        let original = serde_json::to_vec_pretty(&old_config).unwrap();
        std::fs::write(paths.config_file(), &original).unwrap();
        let store = ConfigStore::with_writer(paths.clone(), Arc::new(FailingWriter));

        let result = store.update_launcher(|config| {
            config.language = "en".to_string();
            Ok(())
        });

        assert_eq!(result.unwrap_err(), "injected replace failure");
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), original);
    }

    #[test]
    fn concurrent_updates_preserve_both_launcher_fields() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let store = ConfigStore::new(paths);
        let barrier = Arc::new(Barrier::new(3));

        let settings_store = store.clone();
        let settings_barrier = barrier.clone();
        let settings = std::thread::spawn(move || {
            settings_barrier.wait();
            settings_store
                .update_launcher(|config| {
                    config.language = "en".to_string();
                    Ok(())
                })
                .unwrap();
        });

        let guard_store = store.clone();
        let guard_barrier = barrier.clone();
        let guard = std::thread::spawn(move || {
            guard_barrier.wait();
            guard_store
                .update_launcher(|config| {
                    config.codex_guard.enabled = true;
                    Ok(())
                })
                .unwrap();
        });

        barrier.wait();
        settings.join().unwrap();
        guard.join().unwrap();

        let config = store.load_launcher().unwrap();
        assert_eq!(config.language, "en");
        assert!(config.codex_guard.enabled);
    }
}
