use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::codex_guard::atomic_store::{AtomicFileWriter, PlatformAtomicFileWriter};
use crate::codex_guard::engine::{execute_transaction_batch, TransactionWrite};
use crate::codex_guard::journal::JournalParticipant;
use crate::codex_guard::{AppPaths, GuardParam};

const GUARD_SCHEMA_ENVELOPE_VERSION: u16 = 1;
static NEXT_MIGRATION_BACKUP_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardSchemaEnvelope {
    schema_version: u16,
    params: Vec<GuardParam>,
}

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

pub(crate) struct LauncherTransaction<'a> {
    target: PathBuf,
    original: Option<Vec<u8>>,
    config: LauncherConfig,
    writer: &'a dyn AtomicFileWriter,
}

impl<'a> LauncherTransaction<'a> {
    pub(crate) fn config(&self) -> &LauncherConfig {
        &self.config
    }

    pub(crate) fn config_mut(&mut self) -> &mut LauncherConfig {
        &mut self.config
    }

    pub(crate) fn candidate_bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec_pretty(&self.config).map_err(|error| {
            crate::i18n::trf(
                "Failed to serialize config: {error}",
                &[("error", error.to_string())],
            )
        })
    }

    pub(crate) fn target(&self) -> &Path {
        &self.target
    }

    pub(crate) fn original(&self) -> Option<&[u8]> {
        self.original.as_deref()
    }

    pub(crate) fn writer(&self) -> &dyn AtomicFileWriter {
        self.writer
    }
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

    /// Migrate a legacy Launcher config once, retaining the exact v0 bytes in a
    /// durable backup before replacing the active config.  A failed replacement
    /// leaves the original bytes untouched; callers can retry this operation.
    pub(crate) fn migrate_guard_state(&self) -> Result<bool, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        let path = self.paths.config_file();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(crate::i18n::trf(
                    "Failed to read config file: {error}",
                    &[("error", error.to_string())],
                ))
            }
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            crate::i18n::trf(
                "Failed to parse config file: {error}",
                &[("error", error.to_string())],
            )
        })?;
        let Some(guard) = value
            .get("codex_guard")
            .and_then(serde_json::Value::as_object)
        else {
            return Ok(false);
        };
        let needs_migration = guard
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .is_none_or(|version| version < 1);
        if !needs_migration {
            return Ok(false);
        }

        let mut config: LauncherConfig = serde_json::from_value(value).map_err(|error| {
            crate::i18n::trf(
                "Failed to parse config file: {error}",
                &[("error", error.to_string())],
            )
        })?;
        config.codex_guard.normalize_groups();
        let candidate = serde_json::to_vec_pretty(&config).map_err(|error| {
            crate::i18n::trf(
                "Failed to serialize config: {error}",
                &[("error", error.to_string())],
            )
        })?;

        let backup_dir = self.paths.launcher_root().join("migration-backups");
        std::fs::create_dir_all(&backup_dir).map_err(|error| {
            crate::i18n::trf(
                "Failed to create migration backup: {error}",
                &[("error", error.to_string())],
            )
        })?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let serial = NEXT_MIGRATION_BACKUP_ID.fetch_add(1, Ordering::Relaxed);
        let backup = backup_dir.join(format!("config-v0-{timestamp}-{serial}.json"));
        let mut backup_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
            .map_err(|error| {
                crate::i18n::trf(
                    "Failed to create migration backup: {error}",
                    &[("error", error.to_string())],
                )
            })?;
        use std::io::Write;
        backup_file
            .write_all(&bytes)
            .and_then(|_| backup_file.sync_all())
            .map_err(|error| {
                crate::i18n::trf(
                    "Failed to sync migration backup: {error}",
                    &[("error", error.to_string())],
                )
            })?;

        self.commit_file(
            JournalParticipant::Launcher,
            "config.json",
            Some(bytes),
            candidate,
        )?;
        Ok(true)
    }

    /// Pure migration helper used by contract tests and startup composition.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn migrate_guard_state_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid launcher config: {error}"))?;
        if value
            .get("codex_guard")
            .and_then(serde_json::Value::as_object)
            .is_none()
        {
            return Ok(bytes.to_vec());
        }
        let mut config: LauncherConfig = serde_json::from_value(value)
            .map_err(|error| format!("invalid launcher config: {error}"))?;
        config.codex_guard.normalize_groups();
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())
    }

    pub(crate) fn update_launcher<R>(
        &self,
        update: impl FnOnce(&mut LauncherConfig) -> Result<R, String>,
    ) -> Result<R, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        let (original, mut config) = self.load_launcher_snapshot_unlocked()?;
        let result = update(&mut config)?;
        let bytes = serde_json::to_vec_pretty(&config).map_err(|error| {
            crate::i18n::trf(
                "Failed to serialize config: {error}",
                &[("error", error.to_string())],
            )
        })?;
        self.commit_file(JournalParticipant::Launcher, "config.json", original, bytes)?;
        Ok(result)
    }

    pub(crate) fn with_launcher_transaction<R>(
        &self,
        operation: impl FnOnce(&mut LauncherTransaction<'_>) -> Result<R, String>,
    ) -> Result<R, String> {
        // 联合 Guard 事务会在这个闭包内完成 Codex 写入和 config.json 写入；保持
        // ConfigStore 锁直到批次提交，避免进程内设置写入穿过原始 hash precondition。
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        let (original, config) = self.load_launcher_snapshot_unlocked()?;
        let mut transaction = LauncherTransaction {
            target: self.paths.config_file(),
            original,
            config,
            writer: self.writer.as_ref(),
        };
        operation(&mut transaction)
    }

    /// Mutate the Launcher config and the disk schema under one lock and one
    /// durable journal batch.  Commands that remove or rename schema-backed
    /// files use this boundary so a failed second participant cannot leave the
    /// schema and its persisted lifecycle state out of sync.
    pub(crate) fn with_launcher_and_schema_transaction<R>(
        &self,
        operation: impl FnOnce(&mut LauncherConfig, &mut Vec<GuardParam>) -> Result<R, String>,
    ) -> Result<R, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        let (launcher_original, mut launcher) = self.load_launcher_snapshot_unlocked()?;
        let (schema_original, mut schema) = self.load_guard_schema_snapshot_unlocked()?;
        let result = operation(&mut launcher, &mut schema)?;
        let launcher_candidate = serde_json::to_vec_pretty(&launcher).map_err(|error| {
            crate::i18n::trf(
                "Failed to serialize config: {error}",
                &[("error", error.to_string())],
            )
        })?;
        let schema_candidate = serde_json::to_vec_pretty(&GuardSchemaEnvelope {
            schema_version: GUARD_SCHEMA_ENVELOPE_VERSION,
            params: schema,
        })
        .map_err(|error| {
            crate::i18n::trf(
                "Failed to serialize schema: {error}",
                &[("error", error.to_string())],
            )
        })?;
        let launcher_target = self.paths.config_file();
        let schema_target = self.paths.guard_schema_file();
        let writer = self.writer.as_ref();
        execute_transaction_batch(
            &self.paths,
            vec![
                TransactionWrite {
                    participant: JournalParticipant::Launcher,
                    relative_file: "config.json".to_string(),
                    target: launcher_target,
                    original: launcher_original,
                    candidate_exists: true,
                    candidate: launcher_candidate,
                    writer,
                },
                TransactionWrite {
                    participant: JournalParticipant::LauncherSchema,
                    relative_file: "codex-guard-schema.json".to_string(),
                    target: schema_target,
                    original: schema_original,
                    candidate_exists: true,
                    candidate: schema_candidate,
                    writer,
                },
            ],
        )?;
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
        let (original, mut schema) = self.load_guard_schema_snapshot_unlocked()?;
        let result = update(&mut schema)?;
        let bytes = serde_json::to_vec_pretty(&GuardSchemaEnvelope {
            schema_version: GUARD_SCHEMA_ENVELOPE_VERSION,
            params: schema,
        })
        .map_err(|error| {
            crate::i18n::trf(
                "Failed to serialize schema: {error}",
                &[("error", error.to_string())],
            )
        })?;
        self.commit_file(
            JournalParticipant::LauncherSchema,
            "codex-guard-schema.json",
            original,
            bytes,
        )?;
        Ok(result)
    }

    pub(crate) fn ensure_guard_schema_file(&self, default_bytes: &[u8]) -> Result<(), String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "config store lock is poisoned".to_string())?;
        match std::fs::symlink_metadata(self.paths.guard_schema_file()) {
            Ok(metadata) if metadata.file_type().is_file() => Ok(()),
            Ok(_) => Err("guard schema target is not a regular file".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let params: Vec<GuardParam> =
                    serde_json::from_slice(default_bytes).map_err(|error| {
                        crate::i18n::trf(
                            "Failed to parse built-in schema: {error}",
                            &[("error", error.to_string())],
                        )
                    })?;
                let bytes = serde_json::to_vec_pretty(&GuardSchemaEnvelope {
                    schema_version: GUARD_SCHEMA_ENVELOPE_VERSION,
                    params,
                })
                .map_err(|error| {
                    crate::i18n::trf(
                        "Failed to serialize schema: {error}",
                        &[("error", error.to_string())],
                    )
                })?;
                self.commit_file(
                    JournalParticipant::LauncherSchema,
                    "codex-guard-schema.json",
                    None,
                    bytes,
                )
            }
            Err(error) => Err(crate::i18n::trf(
                "Failed to read schema file: {error}",
                &[("error", error.to_string())],
            )),
        }
    }

    fn load_launcher_unlocked(&self) -> Result<LauncherConfig, String> {
        self.load_launcher_snapshot_unlocked()
            .map(|(_, config)| config)
    }

    fn load_launcher_snapshot_unlocked(&self) -> Result<(Option<Vec<u8>>, LauncherConfig), String> {
        let path = self.paths.config_file();
        let Some(bytes) = (match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(crate::i18n::trf(
                    "Failed to read config file: {error}",
                    &[("error", error.to_string())],
                ));
            }
        }) else {
            return Ok((None, LauncherConfig::default()));
        };
        let mut config: LauncherConfig = serde_json::from_slice(&bytes).map_err(|error| {
            crate::i18n::trf(
                "Failed to parse config file: {error}",
                &[("error", error.to_string())],
            )
        })?;
        config.taskboard_path = strip_unc(&config.taskboard_path);
        Ok((Some(bytes), config))
    }

    fn load_guard_schema_unlocked(&self) -> Result<Vec<GuardParam>, String> {
        self.load_guard_schema_snapshot_unlocked()
            .map(|(_, schema)| schema)
    }

    fn load_guard_schema_snapshot_unlocked(
        &self,
    ) -> Result<(Option<Vec<u8>>, Vec<GuardParam>), String> {
        let path = self.paths.guard_schema_file();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((None, Vec::new()))
            }
            Err(error) => {
                return Err(crate::i18n::trf(
                    "Failed to read schema file: {error}",
                    &[("error", error.to_string())],
                ));
            }
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            crate::i18n::trf(
                "Failed to parse schema file: {error}",
                &[("error", error.to_string())],
            )
        })?;
        if value.is_array() {
            let schema = serde_json::from_value(value).map_err(|error| {
                crate::i18n::trf(
                    "Failed to parse schema file: {error}",
                    &[("error", error.to_string())],
                )
            })?;
            return Ok((Some(bytes), schema));
        }
        let envelope: GuardSchemaEnvelope = serde_json::from_value(value).map_err(|error| {
            crate::i18n::trf(
                "Failed to parse schema envelope: {error}",
                &[("error", error.to_string())],
            )
        })?;
        if envelope.schema_version != GUARD_SCHEMA_ENVELOPE_VERSION {
            return Err("unsupported guard schema version".to_string());
        }
        Ok((Some(bytes), envelope.params))
    }

    fn commit_file(
        &self,
        participant: JournalParticipant,
        relative_file: &str,
        original: Option<Vec<u8>>,
        candidate: Vec<u8>,
    ) -> Result<(), String> {
        let target = match participant {
            JournalParticipant::Launcher if relative_file == "config.json" => {
                self.paths.config_file()
            }
            JournalParticipant::LauncherSchema if relative_file == "codex-guard-schema.json" => {
                self.paths.guard_schema_file()
            }
            _ => return Err("unsupported launcher transaction target".to_string()),
        };
        let write = TransactionWrite {
            participant,
            relative_file: relative_file.to_string(),
            target,
            original,
            candidate_exists: true,
            candidate,
            writer: self.writer.as_ref(),
        };
        execute_transaction_batch(&self.paths, vec![write])
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
    use std::collections::BTreeMap;
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
        let mut params = BTreeMap::new();
        let mut param = GuardParamState::from_lifecycle(
            crate::codex_guard::lifecycle::ParameterLifecycle::Locked,
        );
        param.value = Some(serde_json::json!(false));
        param.last_checked = Some(12345);
        param.last_restored = Some(12340);
        params.insert("features.image_generation".to_string(), param);
        CodexGuardState {
            schema_version: crate::codex_guard::CODEX_GUARD_SCHEMA_VERSION,
            enabled: true,
            params,
            files: Vec::new(),
            groups: Vec::new(),
            pending_lifecycle_migrations: BTreeMap::new(),
            pending_format_migrations: BTreeMap::new(),
            roles: Vec::new(),
            capability_snapshot: None,
        }
    }

    #[test]
    fn merge_settings_preserves_codex_guard() {
        let mut current = LauncherConfig {
            codex_guard: sample_guard_state(),
            taskboard_path: "/old/path".to_string(),
            auto_open: true,
            ..Default::default()
        };

        let settings = LauncherConfig {
            taskboard_path: "/new/path".to_string(),
            auto_open: false,
            cdp_port: 9999,
            ..Default::default()
        };

        merge_settings(&mut current, &settings);

        // 设置类字段被更新
        assert_eq!(current.taskboard_path, "/new/path");
        assert!(!current.auto_open);
        assert_eq!(current.cdp_port, 9999);

        // codex_guard 完整保留，没有被默认值覆盖
        assert!(current.codex_guard.enabled);
        let p = current
            .codex_guard
            .params
            .get("features.image_generation")
            .unwrap();
        assert!(p.applied);
        assert!(p.locked);
        assert_eq!(p.last_checked, Some(12345));
        assert_eq!(p.value, Some(serde_json::json!(false)));
    }

    #[test]
    fn merge_settings_updates_all_setting_fields() {
        let mut current = LauncherConfig {
            codex_guard: sample_guard_state(),
            ..Default::default()
        };

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
        assert!(!current.auto_open);
        assert!(current.separate_window_mode);
        assert!(current.minimize_to_tray_on_close);
        assert_eq!(current.language, "zh-CN");

        // codex_guard 不变
        assert!(current.codex_guard.enabled);
        assert_eq!(current.codex_guard.params.len(), 1);
    }

    #[test]
    fn default_guard_state_is_disabled_and_empty() {
        let cfg = LauncherConfig::default();
        assert!(!cfg.codex_guard.enabled);
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
        let old_config = LauncherConfig {
            language: "zh-CN".to_string(),
            ..Default::default()
        };
        let original = serde_json::to_vec_pretty(&old_config).unwrap();
        std::fs::write(paths.config_file(), &original).unwrap();
        let store = ConfigStore::with_writer(paths.clone(), Arc::new(FailingWriter));

        let result = store.update_launcher(|config| {
            config.language = "en".to_string();
            Ok(())
        });

        assert_eq!(
            result.unwrap_err(),
            "guard transaction failed: replace_failed"
        );
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), original);
        assert!(
            !paths.transaction_root().exists()
                || std::fs::read_dir(paths.transaction_root())
                    .unwrap()
                    .next()
                    .is_none()
        );
    }

    #[test]
    fn failed_replace_preserves_old_guard_schema_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.launcher_root()).unwrap();
        let original = serde_json::to_vec_pretty(&GuardSchemaEnvelope {
            schema_version: GUARD_SCHEMA_ENVELOPE_VERSION,
            params: Vec::new(),
        })
        .unwrap();
        std::fs::write(paths.guard_schema_file(), &original).unwrap();
        let store = ConfigStore::with_writer(paths.clone(), Arc::new(FailingWriter));

        let result = store.update_guard_schema(|schema| {
            schema.push(GuardParam {
                id: "custom.failure".into(),
                label: "Failure".into(),
                label_en: String::new(),
                description: String::new(),
                description_en: String::new(),
                file: "config.toml".into(),
                file_id: String::new(),
                group_id: None,
                apply_mode: "toml_key".into(),
                path: "features.failure".into(),
                value_type: "bool".into(),
                default: serde_json::json!(false),
                default_en: serde_json::Value::Null,
                custom: true,
            });
            Ok(())
        });

        assert_eq!(
            result.unwrap_err(),
            "guard transaction failed: replace_failed"
        );
        assert_eq!(std::fs::read(paths.guard_schema_file()).unwrap(), original);
        assert!(
            !paths.transaction_root().exists()
                || std::fs::read_dir(paths.transaction_root())
                    .unwrap()
                    .next()
                    .is_none()
        );
    }

    #[test]
    fn launcher_transaction_keeps_raw_precondition_and_builds_candidate_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.launcher_root()).unwrap();
        let original_config = LauncherConfig {
            language: "zh-CN".into(),
            ..Default::default()
        };
        let original = serde_json::to_vec_pretty(&original_config).unwrap();
        std::fs::write(paths.config_file(), &original).unwrap();
        let store = ConfigStore::new(paths.clone());

        let candidate = store
            .with_launcher_transaction(|transaction| {
                assert_eq!(transaction.original(), Some(original.as_slice()));
                transaction.config_mut().language = "en".into();
                transaction.candidate_bytes()
            })
            .unwrap();

        assert_ne!(candidate, original);
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

    #[test]
    fn guard_state_migration_writes_v1_envelope_and_backup() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.launcher_root()).unwrap();
        let original = br#"{"codex_guard":{"enabled":true,"params":{"x":{"applied":false,"locked":true}},"files":[],"groups":[]}}"#;
        std::fs::write(paths.config_file(), original).unwrap();
        let store = ConfigStore::new(paths.clone());

        assert!(store.migrate_guard_state().unwrap());
        let migrated: serde_json::Value =
            serde_json::from_slice(&std::fs::read(paths.config_file()).unwrap()).unwrap();
        assert_eq!(migrated["codex_guard"]["schema_version"], 1);
        assert_eq!(
            migrated["codex_guard"]["pending_lifecycle_migrations"]["x"]["locked"],
            true
        );
        let backups = std::fs::read_dir(paths.launcher_root().join("migration-backups"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(backups[0].path()).unwrap(), original);
        assert!(!store.migrate_guard_state().unwrap());
    }

    #[test]
    fn guard_schema_updates_use_a_versioned_envelope_but_read_legacy_arrays() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.launcher_root()).unwrap();
        let legacy = serde_json::json!([{
            "id": "custom.demo",
            "label": "Demo",
            "file": "config.toml",
            "apply_mode": "toml_key",
            "path": "features.demo",
            "value_type": "bool",
            "default": false,
            "custom": true
        }]);
        std::fs::write(
            paths.guard_schema_file(),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let store = ConfigStore::new(paths.clone());
        assert_eq!(store.load_guard_schema().unwrap().len(), 1);
        store
            .update_guard_schema(|schema| {
                schema[0].description = "changed".to_string();
                Ok(())
            })
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(paths.guard_schema_file()).unwrap()).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert!(value["params"].is_array());
    }
}
