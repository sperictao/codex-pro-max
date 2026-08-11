//! Codex 配置看守：schema 驱动的 ~/.codex 参数托管、锁定与漂移恢复。
//! 词汇与语义边界见仓库 CONTEXT.md 与 docs/adr/0001。

pub(crate) mod atomic_store;
pub(crate) mod audit;
pub(crate) mod audit_commands;
pub(crate) mod audit_runner;
pub(crate) mod audit_sqlite;
mod backup;
pub(crate) mod batch;
pub(crate) mod capability;
mod commands;
pub(crate) mod contracts;
mod coordinator;
pub(crate) mod engine;
mod files;
pub(crate) mod format;
pub(crate) mod journal;
pub(crate) mod lifecycle;
mod markdown_block;
pub(crate) mod model;
pub(crate) mod operation_audit;
pub(crate) mod ownership;
mod paths;
mod poll;
pub(crate) mod role_commands;
pub(crate) mod roles;
pub(crate) mod roles_store;
mod schema;
mod toml_ops;
pub(crate) mod transaction;
mod validate;
mod view;

pub use audit_commands::*;
pub use commands::*;
pub(crate) use coordinator::GuardCoordinator;
pub use coordinator::GuardRecoveryStatus;
pub(crate) use engine::recover_pending_transactions;
pub use model::GuardFileFormat;
pub(crate) use paths::AppPaths;
pub use poll::poll_loop;
pub use role_commands::*;
pub use roles_store::ManagedRoleState;
pub(crate) use transaction::is_recovery_blocking_error;

use schema::pick_i18n;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

use self::lifecycle::ParameterLifecycle;
use self::roles::CapabilitySnapshot;

/// Persisted Guard state envelope version.  Missing `schema_version` is the v0
/// boolean state and is migrated at the deserialization boundary.
pub const CODEX_GUARD_SCHEMA_VERSION: u16 = 1;

// ============ schema 与状态类型 ============

/// schema 中的一条托管参数
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GuardParam {
    pub id: String,
    pub label: String,
    /// 英文 label；空 = 无英文资源，落回 label 原文（自定义参数即如此）
    #[serde(default)]
    pub label_en: String,
    #[serde(default)]
    pub description: String,
    /// 英文 description；空 = 落回 description 原文
    #[serde(default)]
    pub description_en: String,
    /// 相对 ~/.codex 的路径，如 config.toml / AGENTS.md / agents/default.toml
    pub file: String,
    /// 稳定的物理文件身份；旧 schema 缺失时由加载边界按路径生成兼容 ID。
    #[serde(default, rename = "fileId", alias = "file_id")]
    pub file_id: String,
    /// 逻辑组 ID；旧配置缺失时归入 uncategorized。
    #[serde(default, rename = "groupId", alias = "group")]
    pub group_id: Option<String>,
    /// toml_key | toml_absent | file_overwrite | markdown_block
    pub apply_mode: String,
    /// 点分 TOML 路径（toml 模式用）
    #[serde(default)]
    pub path: String,
    /// bool | int | string | text | none
    #[serde(default)]
    pub value_type: String,
    #[serde(default)]
    pub default: serde_json::Value,
    /// 英文 default（写入内容本身双语时用）；Null = 内容不随界面语言变化
    #[serde(default)]
    pub default_en: serde_json::Value,
    /// 是否为用户自定义参数（非内置）；自定义参数可删除
    #[serde(default)]
    pub custom: bool,
}

impl GuardParam {
    pub fn effective_file_id(&self) -> String {
        if self.file_id.is_empty() {
            format!("path:{}", self.file)
        } else {
            self.file_id.clone()
        }
    }

    /// 当前界面语言下的 label（default 值是写入 codex 的内容，不在翻译范围）
    pub fn localized_label(&self) -> &str {
        pick_i18n(&self.label, &self.label_en, crate::i18n::current())
    }

    pub fn localized_description(&self) -> &str {
        pick_i18n(
            &self.description,
            &self.description_en,
            crate::i18n::current(),
        )
    }
}

/// v0 中 `applied`/`locked` 两个布尔值无法表达的组合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingLifecycleMigration {
    pub applied: bool,
    pub locked: bool,
}

/// A persisted file entry whose physical format cannot be selected safely yet.
///
/// The marker deliberately keeps only the file identity and a bounded list of
/// candidate formats.  It never silently chooses `plain_text`; the file stays
/// out of all write plans until the user resolves it explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingFormatMigration {
    pub id: String,
    pub name: String,
    pub file: String,
    pub builtin: bool,
    pub detection: Option<DetectRecord>,
    pub raw_format: Option<String>,
    pub candidates: Vec<GuardFileFormat>,
}

/// 用户在生命周期迁移向导中作出的选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleMigrationChoice {
    Disabled,
    Apply,
}

/// 单个参数的托管状态，持久化在 LauncherConfig.codex_guard.params。
///
/// `applied`/`locked` 保留为旧 IPC 的兼容投影；序列化时额外写出合法的
/// `lifecycle` 枚举，反序列化时以枚举为准并同步布尔值。
#[derive(Debug, Clone, Default, specta::Type)]
pub struct GuardParamState {
    /// 用户改后的值；None = 用 schema 推荐值
    pub value: Option<serde_json::Value>,
    pub applied: bool,
    pub locked: bool,
    pub last_checked: Option<u64>,
    pub last_restored: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GuardParamStateWire {
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(default)]
    lifecycle: Option<ParameterLifecycle>,
    #[serde(default)]
    applied: bool,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    last_checked: Option<u64>,
    #[serde(default)]
    last_restored: Option<u64>,
}

impl<'de> Deserialize<'de> for GuardParamState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = GuardParamStateWire::deserialize(deserializer)?;
        let (applied, locked) = match wire.lifecycle {
            Some(lifecycle) => (lifecycle.is_enabled(), lifecycle.is_locked()),
            None => match ParameterLifecycle::from_legacy_flags(wire.applied, wire.locked) {
                Ok(lifecycle) => (lifecycle.is_enabled(), lifecycle.is_locked()),
                Err(_) => (false, false),
            },
        };
        Ok(Self {
            value: wire.value,
            applied,
            locked,
            last_checked: wire.last_checked,
            last_restored: wire.last_restored,
        })
    }
}

impl Serialize for GuardParamState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Wire<'a> {
            value: &'a Option<serde_json::Value>,
            lifecycle: ParameterLifecycle,
            applied: bool,
            locked: bool,
            last_checked: Option<u64>,
            last_restored: Option<u64>,
        }
        Wire {
            value: &self.value,
            lifecycle: self.lifecycle(),
            applied: self.applied,
            locked: self.locked,
            last_checked: self.last_checked,
            last_restored: self.last_restored,
        }
        .serialize(serializer)
    }
}

impl GuardParamState {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn from_lifecycle(lifecycle: ParameterLifecycle) -> Self {
        Self {
            applied: lifecycle.is_enabled(),
            locked: lifecycle.is_locked(),
            ..Self::default()
        }
    }

    pub fn set_lifecycle(&mut self, lifecycle: ParameterLifecycle) {
        self.applied = lifecycle.is_enabled();
        self.locked = lifecycle.is_locked();
    }

    pub fn lifecycle(&self) -> ParameterLifecycle {
        ParameterLifecycle::from_flags(self.applied, self.locked)
            .unwrap_or(ParameterLifecycle::Disabled)
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct CodexGuardState {
    pub schema_version: u16,
    pub enabled: bool,
    pub params: BTreeMap<String, GuardParamState>,
    /// 看守目标文件列表（内置 + 自定义）
    pub files: Vec<GuardFile>,
    /// 逻辑参数组定义；缺失时由 schema 参数的 group_id 派生。
    pub groups: Vec<GuardGroup>,
    /// v0 非法布尔组合等待显式迁移选择；未决项不会进入批量/轮询写计划。
    pub pending_lifecycle_migrations: BTreeMap<String, PendingLifecycleMigration>,
    /// File entries with a missing/unknown format.  These block Guard writes
    /// until `guard_file_format_migration_resolve` chooses one candidate.
    pub pending_format_migrations: BTreeMap<String, PendingFormatMigration>,
    #[serde(default)]
    #[specta(skip)]
    pub roles: Vec<ManagedRoleState>,
    /// Last successful capability probe. It is an offline read cache only;
    /// role drafts are never stored here.
    #[serde(default)]
    #[specta(skip)]
    pub capability_snapshot: Option<CapabilitySnapshot>,
}

#[derive(Debug, Deserialize)]
struct CodexGuardStateWire {
    #[serde(default)]
    schema_version: Option<u16>,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    files: Vec<GuardFileWire>,
    #[serde(default)]
    groups: Vec<GuardGroup>,
    #[serde(default)]
    pending_lifecycle_migrations: BTreeMap<String, PendingLifecycleMigration>,
    #[serde(default)]
    pending_format_migrations: BTreeMap<String, PendingFormatMigration>,
    #[serde(default)]
    roles: Vec<ManagedRoleState>,
    #[serde(default)]
    capability_snapshot: Option<CapabilitySnapshot>,
}

impl<'de> Deserialize<'de> for CodexGuardState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CodexGuardStateWire::deserialize(deserializer)?;
        let version = wire.schema_version.unwrap_or(0);
        if version > CODEX_GUARD_SCHEMA_VERSION {
            return Err(D::Error::custom("unsupported codex guard schema version"));
        }
        let mut pending_lifecycle_migrations = wire.pending_lifecycle_migrations;
        let mut pending_format_migrations = wire.pending_format_migrations;
        let mut params = BTreeMap::new();
        for (id, value) in wire.params {
            let invalid = value.get("lifecycle").is_none()
                && value.get("applied").and_then(serde_json::Value::as_bool) == Some(false)
                && value.get("locked").and_then(serde_json::Value::as_bool) == Some(true);
            let parsed: GuardParamState = serde_json::from_value(value)
                .map_err(|error| D::Error::custom(error.to_string()))?;
            if invalid {
                pending_lifecycle_migrations.insert(
                    id.clone(),
                    PendingLifecycleMigration {
                        applied: false,
                        locked: true,
                    },
                );
            }
            params.insert(id, parsed);
        }
        let mut files = Vec::new();
        for file in wire.files {
            match file.resolve_format().map_err(D::Error::custom)? {
                Ok(file) => files.push(file),
                Err(pending) => {
                    pending_format_migrations.insert(pending.id.clone(), pending);
                }
            }
        }
        let mut state = Self {
            schema_version: CODEX_GUARD_SCHEMA_VERSION,
            enabled: wire.enabled,
            params,
            files,
            groups: wire.groups,
            pending_lifecycle_migrations,
            pending_format_migrations,
            roles: wire.roles,
            capability_snapshot: wire.capability_snapshot,
        };
        state.normalize_groups();
        Ok(state)
    }
}

impl Default for CodexGuardState {
    fn default() -> Self {
        Self {
            schema_version: CODEX_GUARD_SCHEMA_VERSION,
            enabled: false,
            params: BTreeMap::new(),
            files: Vec::new(),
            groups: builtin_groups(),
            pending_lifecycle_migrations: BTreeMap::new(),
            pending_format_migrations: BTreeMap::new(),
            roles: Vec::new(),
            capability_snapshot: None,
        }
    }
}

impl CodexGuardState {
    /// Add immutable built-ins while retaining custom group order.
    pub fn normalize_groups(&mut self) {
        let builtins = builtin_groups();
        let mut groups = builtins
            .iter()
            .map(|builtin| {
                let mut group = self
                    .groups
                    .iter()
                    .find(|group| group.id == builtin.id)
                    .cloned()
                    .unwrap_or_else(|| builtin.clone());
                group.name = builtin.name.clone();
                group.builtin = true;
                group.order = builtin.order;
                group
            })
            .collect::<Vec<_>>();
        let mut seen = groups
            .iter()
            .map(|group| group.id.clone())
            .collect::<std::collections::HashSet<_>>();
        for group in &self.groups {
            if !builtins.iter().any(|builtin| builtin.id == group.id)
                && seen.insert(group.id.clone())
            {
                groups.push(group.clone());
            }
        }
        self.groups = groups;
        self.schema_version = CODEX_GUARD_SCHEMA_VERSION;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GuardGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub order: u32,
}

pub fn builtin_groups() -> Vec<GuardGroup> {
    vec![
        GuardGroup {
            id: "subagent-optimization".to_string(),
            name: "Subagent optimization".to_string(),
            builtin: true,
            order: 0,
        },
        GuardGroup {
            id: "general".to_string(),
            name: "General".to_string(),
            builtin: true,
            order: 1,
        },
    ]
}

/// Canonicalize group IDs from pre-v1 schema entries.  Built-in parameters are
/// never movable, so the mapping is deterministic; custom parameters without a
/// group remain in the hidden-until-used `uncategorized` bucket.
pub fn canonical_group_id(param_id: &str, raw_group: Option<&str>, custom: bool) -> String {
    if custom {
        return raw_group.unwrap_or("uncategorized").to_string();
    }
    if param_id == "features.image_generation" {
        return "general".to_string();
    }
    if param_id.starts_with("features.multi_agent_v2.")
        || matches!(param_id, "agents-v1-remove" | "agents-md-subagent-section")
        || raw_group == Some("subagent-operations")
    {
        return "subagent-optimization".to_string();
    }
    raw_group.unwrap_or("general").to_string()
}

/// 看守目标文件
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct GuardFile {
    pub id: String,
    pub name: String,
    /// 相对 ~/.codex 的路径
    pub file: String,
    /// toml | json | markdown | plain_text
    pub format: GuardFileFormat,
    #[serde(default)]
    pub builtin: bool,
    /// 上次路径检测记录；None = 从未检测（检测走文件系统，落盘后不再重复扫）
    #[serde(default)]
    pub detection: Option<DetectRecord>,
}

#[derive(Debug, Deserialize)]
struct GuardFileWire {
    id: String,
    name: String,
    file: String,
    #[serde(default)]
    format: Option<serde_json::Value>,
    #[serde(default)]
    builtin: bool,
    #[serde(default)]
    detection: Option<DetectRecord>,
}

impl GuardFileWire {
    fn resolve_format(self) -> Result<Result<GuardFile, PendingFormatMigration>, String> {
        let raw_format = self
            .format
            .as_ref()
            .and_then(|value| value.as_str())
            .map(str::to_string);
        if let Some(value) = self.format.as_ref().and_then(|value| value.as_str()) {
            if let Ok(format) = serde_json::from_value::<GuardFileFormat>(
                serde_json::Value::String(value.to_string()),
            ) {
                return Ok(Ok(GuardFile {
                    id: self.id,
                    name: self.name,
                    file: self.file,
                    format,
                    builtin: self.builtin,
                    detection: self.detection,
                }));
            }
        }
        let candidates = infer_file_formats(&self.id, &self.file, self.builtin);
        if candidates.len() == 1 {
            return Ok(Ok(GuardFile {
                id: self.id,
                name: self.name,
                file: self.file,
                format: candidates[0],
                builtin: self.builtin,
                detection: self.detection,
            }));
        }
        Ok(Err(PendingFormatMigration {
            id: self.id,
            name: self.name,
            file: self.file,
            builtin: self.builtin,
            detection: self.detection,
            raw_format,
            candidates,
        }))
    }
}

fn infer_file_formats(id: &str, file: &str, _builtin: bool) -> Vec<GuardFileFormat> {
    let known = match id {
        "builtin.config-toml" | "builtin.default-toml" => Some(GuardFileFormat::Toml),
        "builtin.agents-md" => Some(GuardFileFormat::Markdown),
        _ => None,
    };
    if let Some(format) = known {
        return vec![format];
    }
    let extension = std::path::Path::new(file)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    match extension.as_deref() {
        Some("toml") => vec![GuardFileFormat::Toml],
        Some("json") => vec![GuardFileFormat::Json],
        Some("md" | "markdown") => vec![GuardFileFormat::Markdown],
        _ => vec![
            GuardFileFormat::Toml,
            GuardFileFormat::Json,
            GuardFileFormat::Markdown,
            GuardFileFormat::PlainText,
        ],
    }
}

/// 一次路径检测的结果记录
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DetectRecord {
    /// 检测到的相对 ~/.codex 路径；None = 未找到该文件
    pub path: Option<String>,
    pub at: u64,
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_boolean_state_migrates_without_preserving_invalid_lock() {
        let state: CodexGuardState = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "params": {
                "valid": {"applied": true, "locked": true},
                "invalid": {"applied": false, "locked": true}
            },
            "files": [],
            "groups": []
        }))
        .unwrap();
        assert_eq!(state.schema_version, CODEX_GUARD_SCHEMA_VERSION);
        assert_eq!(
            state.params["valid"].lifecycle(),
            ParameterLifecycle::Locked
        );
        assert_eq!(
            state.params["invalid"].lifecycle(),
            ParameterLifecycle::Disabled
        );
        assert!(!state.params["invalid"].locked);
        assert_eq!(
            state.pending_lifecycle_migrations["invalid"],
            PendingLifecycleMigration {
                applied: false,
                locked: true
            }
        );
        assert!(state.groups.iter().all(|group| group.builtin));
    }

    #[test]
    fn v0_fixture_migrates_the_invalid_combination_to_pending_resolution() {
        let value: serde_json::Value =
            serde_json::from_slice(include_bytes!("fixtures/migration/launcher-v0.json")).unwrap();
        let state: CodexGuardState =
            serde_json::from_value(value["codex_guard"].clone()).expect("v0 fixture guard state");
        assert_eq!(
            state.params["disabled"].lifecycle(),
            ParameterLifecycle::Disabled
        );
        assert_eq!(
            state.params["applied"].lifecycle(),
            ParameterLifecycle::Applied
        );
        assert_eq!(
            state.params["locked"].lifecycle(),
            ParameterLifecycle::Locked
        );
        assert_eq!(
            state.params["invalid"].lifecycle(),
            ParameterLifecycle::Disabled
        );
        assert_eq!(
            state.pending_lifecycle_migrations["invalid"],
            PendingLifecycleMigration {
                applied: false,
                locked: true,
            }
        );
    }

    #[test]
    fn state_serializes_as_v1_with_lifecycle_projection() {
        let mut state = CodexGuardState::default();
        state.params.insert(
            "demo".to_string(),
            GuardParamState::from_lifecycle(ParameterLifecycle::Applied),
        );
        let value = serde_json::to_value(state).unwrap();
        assert_eq!(value["schema_version"], CODEX_GUARD_SCHEMA_VERSION);
        assert_eq!(value["params"]["demo"]["lifecycle"], "applied");
        assert_eq!(value["params"]["demo"]["applied"], true);
        assert_eq!(value["params"]["demo"]["locked"], false);
    }

    /// 语义相同的状态必须序列化成相同字节。否则每次保存都被判为"已变化"，
    /// 空操作也会走完整事务并消耗用户可见的 durable backup 配额。
    #[test]
    fn state_serialization_bytes_are_stable_across_reloads() {
        let ids = [
            "zeta", "alpha", "mike", "bravo", "yankee", "charlie", "x-ray", "delta",
        ];
        let mut first = CodexGuardState::default();
        for id in ids {
            first
                .params
                .insert(id.to_string(), GuardParamState::default());
            first.pending_lifecycle_migrations.insert(
                id.to_string(),
                PendingLifecycleMigration {
                    applied: false,
                    locked: true,
                },
            );
        }

        // 反向插入同一批 key：语义等价，但哈希表的迭代顺序不由插入顺序决定。
        let mut second = CodexGuardState::default();
        for id in ids.iter().rev() {
            second
                .params
                .insert((*id).to_string(), GuardParamState::default());
            second.pending_lifecycle_migrations.insert(
                (*id).to_string(),
                PendingLifecycleMigration {
                    applied: false,
                    locked: true,
                },
            );
        }

        assert_eq!(
            serde_json::to_string_pretty(&first).unwrap(),
            serde_json::to_string_pretty(&second).unwrap(),
            "identical guard state must serialize to identical bytes"
        );
    }

    #[test]
    fn unknown_state_version_is_rejected() {
        let result = serde_json::from_value::<CodexGuardState>(serde_json::json!({
            "schema_version": 99,
            "params": {}
        }));
        assert!(result.is_err());
    }

    #[test]
    fn legacy_md_alias_resolves_without_a_pending_marker() {
        let state: CodexGuardState = serde_json::from_value(serde_json::json!({
            "schema_version": 0,
            "files": [{
                "id": "custom.readme",
                "name": "Readme",
                "file": "docs/readme.md",
                "format": "md"
            }]
        }))
        .unwrap();
        assert_eq!(state.files[0].format, GuardFileFormat::Markdown);
        assert!(state.pending_format_migrations.is_empty());
    }

    #[test]
    fn missing_format_with_unique_extension_is_inferred() {
        let state: CodexGuardState = serde_json::from_value(serde_json::json!({
            "schema_version": 0,
            "files": [{
                "id": "custom.settings",
                "name": "Settings",
                "file": "settings.json"
            }]
        }))
        .unwrap();
        assert_eq!(state.files[0].format, GuardFileFormat::Json);
        assert!(state.pending_format_migrations.is_empty());
    }

    #[test]
    fn unknown_or_ambiguous_format_becomes_pending_and_roundtrips() {
        let state: CodexGuardState = serde_json::from_value(serde_json::json!({
            "schema_version": 0,
            "files": [{
                "id": "custom.opaque",
                "name": "Opaque",
                "file": "settings",
                "format": "yaml"
            }]
        }))
        .unwrap();
        let pending = &state.pending_format_migrations["custom.opaque"];
        assert_eq!(pending.raw_format.as_deref(), Some("yaml"));
        assert_eq!(pending.candidates.len(), 4);
        assert!(state.files.is_empty());

        let encoded = serde_json::to_value(&state).unwrap();
        let restored: CodexGuardState = serde_json::from_value(encoded).unwrap();
        assert_eq!(
            restored.pending_format_migrations,
            state.pending_format_migrations
        );
    }
}
