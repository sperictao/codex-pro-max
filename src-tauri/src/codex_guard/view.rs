//! 视图组装：把 schema + 托管状态 + 文件实况拼成给前端的 GuardView

use serde::Serialize;

use crate::config::ConfigStore;

use super::engine::{check, expected_of};
use super::files::load_files;
use super::schema::load_schema;
use super::AppPaths;

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ParamView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub apply_mode: String,
    pub value_type: String,
    pub path: String,
    pub default: serde_json::Value,
    pub value: serde_json::Value,
    pub applied: bool,
    pub locked: bool,
    pub actual: Option<String>,
    /// match | drift | missing | error
    pub status: String,
    pub error: Option<String>,
    pub last_checked: Option<u64>,
    pub last_restored: Option<u64>,
    pub custom: bool,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub id: String,
    pub name: String,
    pub file: String,
    pub format: String,
    pub builtin: bool,
    pub error: Option<String>,
    pub params: Vec<ParamView>,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GuardView {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u16,
    pub enabled: bool,
    pub groups: Vec<GroupView>,
}

pub fn build_view(store: &ConfigStore, paths: &AppPaths) -> Result<GuardView, String> {
    let cfg = store.load_launcher()?;
    let schema = load_schema(store)?;
    let files = load_files(store)?;

    let mut groups: Vec<GroupView> = Vec::new();
    for f in &files {
        let mut group_params: Vec<ParamView> = Vec::new();
        let mut group_error: Option<String> = None;

        for p in schema.iter().filter(|p| p.file == f.file) {
            let state = cfg.codex_guard.params.get(&p.id);
            let expected = expected_of(p, state);
            let c = check(paths, p, &expected);
            if group_error.is_none() && c.error.is_some() {
                group_error = c.error.clone();
            }
            group_params.push(ParamView {
                id: p.id.clone(),
                label: p.localized_label().to_string(),
                description: p.localized_description().to_string(),
                apply_mode: p.apply_mode.clone(),
                value_type: p.value_type.clone(),
                path: p.path.clone(),
                default: p.default.clone(),
                value: expected,
                applied: state.is_some_and(|s| s.applied),
                locked: state.is_some_and(|s| s.locked),
                actual: c.actual,
                status: c.status,
                error: c.error,
                last_checked: state.and_then(|s| s.last_checked),
                last_restored: state.and_then(|s| s.last_restored),
                custom: p.custom,
            });
        }

        groups.push(GroupView {
            id: f.id.clone(),
            name: f.name.clone(),
            file: f.file.clone(),
            format: f.format.clone(),
            builtin: f.builtin,
            error: group_error,
            params: group_params,
        });
    }
    Ok(GuardView {
        schema_version: super::contracts::GUARD_CONTRACT_SCHEMA_VERSION,
        enabled: cfg.codex_guard.enabled,
        groups,
    })
}
