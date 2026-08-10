//! Rust 单一来源的 Guard IPC 合同与 TypeScript 导出入口。

use std::path::Path;

use serde::Serialize;
use specta::Type;
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, collect_events, Builder};

// `#[tauri::command]` publishes helper macros at the crate root. The collect macro expands
// in this nested module, so bring those generated names into its lexical scope.
use crate::*;

use super::{
    guard_add_custom_param, guard_add_file, guard_apply, guard_detect_file, guard_get_files,
    guard_get_recovery_status, guard_get_schema_file_path, guard_get_view,
    guard_relativize_picked_path,
    guard_remove_custom_param, guard_remove_file, guard_set_applied, guard_set_enabled,
    guard_retry_recovery, guard_set_locked, guard_set_value, guard_update_file, CodexGuardState,
    DetectRecord, GuardFile, GuardFileFormat, GuardParam, GuardParamState, GuardRecoveryStatus,
};

pub const GUARD_CONTRACT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GuardContractConstants {
    pub schema_version: u16,
    pub apply_modes: Vec<&'static str>,
    pub value_types: Vec<&'static str>,
    pub file_formats: Vec<&'static str>,
    pub param_statuses: Vec<&'static str>,
}

pub const GUARD_APPLY_MODES: [&str; 4] = [
    "toml_key",
    "toml_absent",
    "file_overwrite",
    "markdown_block",
];
pub const GUARD_VALUE_TYPES: [&str; 5] = ["bool", "int", "string", "text", "none"];
pub const GUARD_FILE_FORMATS: [&str; 4] = ["toml", "json", "markdown", "plain_text"];
pub const GUARD_PARAM_STATUSES: [&str; 4] = ["match", "drift", "missing", "error"];

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            guard_get_view,
            guard_set_enabled,
            guard_set_value,
            guard_apply,
            guard_set_applied,
            guard_set_locked,
            guard_add_custom_param,
            guard_remove_custom_param,
            guard_get_schema_file_path,
            guard_get_files,
            guard_add_file,
            guard_update_file,
            guard_remove_file,
            guard_detect_file,
            guard_relativize_picked_path,
            guard_get_recovery_status,
            guard_retry_recovery,
        ])
        .events(collect_events![])
        .typ::<GuardParam>()
        .typ::<GuardParamState>()
        .typ::<CodexGuardState>()
        .typ::<GuardFile>()
        .typ::<GuardFileFormat>()
        .typ::<DetectRecord>()
        .typ::<GuardRecoveryStatus>()
        .constant(
            "GUARD_CONTRACT",
            GuardContractConstants {
                schema_version: GUARD_CONTRACT_SCHEMA_VERSION,
                apply_modes: GUARD_APPLY_MODES.to_vec(),
                value_types: GUARD_VALUE_TYPES.to_vec(),
                file_formats: GUARD_FILE_FORMATS.to_vec(),
                param_statuses: GUARD_PARAM_STATUSES.to_vec(),
            },
        )
}

pub fn export_to(path: impl AsRef<Path>) -> Result<(), String> {
    let path = path.as_ref();
    builder()
        .export(
            Typescript::default()
                .header("// @ts-nocheck")
                .bigint(BigIntExportBehavior::Number),
            path,
        )
        .map_err(|error| error.to_string())?;

    let generated = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let trailing_newline = generated.ends_with('\n');
    let mut normalized = generated
        .lines()
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n");
    if trailing_newline {
        normalized.push('\n');
    }
    if normalized != generated {
        std::fs::write(path, normalized).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
fn fixture_view() -> super::view::GuardView {
    super::view::GuardView {
        schema_version: GUARD_CONTRACT_SCHEMA_VERSION,
        enabled: false,
        groups: vec![super::view::GroupView {
            id: "config".to_string(),
            name: "Codex config".to_string(),
            file: "config.toml".to_string(),
            format: GuardFileFormat::Toml,
            builtin: true,
            error: None,
            params: vec![super::view::ParamView {
                id: "demo".to_string(),
                label: "Demo".to_string(),
                description: String::new(),
                apply_mode: "toml_key".to_string(),
                value_type: "bool".to_string(),
                path: "features.demo".to_string(),
                default: serde_json::json!(false),
                value: serde_json::json!(false),
                applied: false,
                locked: false,
                actual: None,
                status: "match".to_string(),
                error: None,
                last_checked: None,
                last_restored: None,
                custom: false,
            }],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::fixture_view;

    #[test]
    fn guard_view_fixture_is_stable() {
        let mut actual = serde_json::to_vec(&fixture_view()).expect("serialize GuardView fixture");
        actual.push(b'\n');
        assert_eq!(
            actual,
            include_bytes!("fixtures/contracts/guard-view.json"),
            "GuardView JSON changed; update the fixture only with an intentional contract change",
        );
    }
}
