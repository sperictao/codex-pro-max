//! Codex 配置看守：schema 驱动的 ~/.codex 参数托管、锁定与漂移恢复。
//! 词汇与语义边界见仓库 CONTEXT.md 与 docs/adr/0001。

mod backup;
pub(crate) mod atomic_store;
mod commands;
pub(crate) mod contracts;
mod coordinator;
mod engine;
mod files;
pub(crate) mod format;
pub(crate) mod journal;
mod markdown_block;
pub(crate) mod model;
pub(crate) mod ownership;
mod paths;
mod poll;
mod schema;
mod toml_ops;
pub(crate) mod transaction;
mod validate;
mod view;

pub use commands::*;
pub use poll::poll_loop;
pub(crate) use coordinator::GuardCoordinator;
pub use coordinator::GuardRecoveryStatus;
pub(crate) use engine::recover_pending_transactions;
pub(crate) use paths::AppPaths;
pub use model::GuardFileFormat;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use schema::pick_i18n;

// ============ schema 与状态类型 ============

/// schema 中的一条托管参数
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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
    /// 当前界面语言下的 label（default 值是写入 codex 的内容，不在翻译范围）
    pub fn localized_label(&self) -> &str {
        pick_i18n(&self.label, &self.label_en, crate::i18n::current())
    }

    pub fn localized_description(&self) -> &str {
        pick_i18n(&self.description, &self.description_en, crate::i18n::current())
    }
}

/// 单个参数的托管状态，持久化在 LauncherConfig.codex_guard.params
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct GuardParamState {
    /// 用户改后的值；None = 用 schema 推荐值
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub applied: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub last_checked: Option<u64>,
    #[serde(default)]
    pub last_restored: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CodexGuardState {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub params: HashMap<String, GuardParamState>,
    /// 看守目标文件列表（内置 + 自定义）
    #[serde(default)]
    pub files: Vec<GuardFile>,
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

/// 一次路径检测的结果记录
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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
