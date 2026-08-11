//! Rust 单一来源的 Guard IPC 合同与 TypeScript 导出入口。

use std::path::Path;

use serde::Serialize;
use specta::Type;
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, collect_events, Builder};

// `#[tauri::command]` publishes helper macros at the crate root. The collect macro expands
// in this nested module, so bring those generated names into its lexical scope.
use crate::*;

use super::audit::{
    AgentAudit, AuditNotice, AuditSourceSupport, AuditVerdict, ManagedRoleSnapshot,
    OperationAuditPhase, OperationAuditRecord, OperationAuditResult, OutboundObservation,
    ParentDispatchAudit, SubagentAuditResult, TurnAudit,
};
use super::audit_commands::{guard_operation_audit_list, guard_run_subagent_audit};
use super::batch::{
    BatchAction, BatchOutcome, BatchPreview, BatchReport, BatchRequest, BatchScope,
    GuardOperationPhase, GuardOperationProgress,
};
use super::model::{DiagnosticCode, DiagnosticParams, DiagnosticSeverity, ValidationDiagnostic};
use super::role_commands::{
    guard_capability_get, guard_capability_refresh, guard_role_adopt, guard_role_copy,
    guard_role_delete, guard_role_discover, guard_role_get, guard_role_list,
    guard_role_migration_plan, guard_role_migration_resolve, guard_role_reorder, guard_role_save,
    guard_role_stop_managing, CapabilityModelDto, CapabilitySnapshotDto, RoleCopyInput,
    RoleDeleteReport, RoleDetailDto, RoleDiscoveryDiagnosticDto, RoleDiscoveryDto,
    RoleDiscoveryItemDto, RoleListDto, RoleMigrationDto, RoleMigrationResolveInput,
    RoleReorderInput, RoleSaveInput, RoleSummaryDto,
};
use super::view::{
    ActionEligibilityView, ActionStatesView, GroupFileRef, GroupView, GuardRuntimeState, ParamView,
    PhysicalFileView,
};
use super::{
    guard_add_custom_param, guard_add_file, guard_apply, guard_detect_file,
    guard_file_format_migration_resolve, guard_get_files, guard_get_recovery_status,
    guard_get_schema_file_path, guard_get_view, guard_group_create, guard_group_delete,
    guard_group_rename, guard_group_reorder, guard_lifecycle_migration_resolve,
    guard_parameter_move, guard_relativize_picked_path, guard_remove_custom_param,
    guard_remove_file, guard_retry_recovery, guard_set_applied, guard_set_enabled,
    guard_set_locked, guard_set_value, guard_update_file, CodexGuardState, DetectRecord, GuardFile,
    GuardFileFormat, GuardGroup, GuardParam, GuardParamState, GuardRecoveryStatus,
    LifecycleMigrationChoice, PendingFormatMigration,
};
use super::{guard_execute_batch, guard_preview_batch};

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
            guard_preview_batch,
            guard_execute_batch,
            guard_group_create,
            guard_group_rename,
            guard_group_reorder,
            guard_group_delete,
            guard_parameter_move,
            guard_lifecycle_migration_resolve,
            guard_file_format_migration_resolve,
            guard_role_get,
            guard_role_list,
            guard_role_save,
            guard_role_copy,
            guard_role_discover,
            guard_role_adopt,
            guard_role_reorder,
            guard_role_stop_managing,
            guard_role_delete,
            guard_capability_get,
            guard_capability_refresh,
            guard_role_migration_plan,
            guard_role_migration_resolve,
            guard_run_subagent_audit,
            guard_operation_audit_list,
        ])
        .events(collect_events![GuardOperationProgress])
        .typ::<GuardParam>()
        .typ::<GuardParamState>()
        .typ::<CodexGuardState>()
        .typ::<GuardFile>()
        .typ::<GuardFileFormat>()
        .typ::<DetectRecord>()
        .typ::<GuardRecoveryStatus>()
        .typ::<PendingFormatMigration>()
        .typ::<GuardGroup>()
        .typ::<GuardRuntimeState>()
        .typ::<ActionEligibilityView>()
        .typ::<ActionStatesView>()
        .typ::<PhysicalFileView>()
        .typ::<GroupFileRef>()
        .typ::<GroupView>()
        .typ::<ParamView>()
        .typ::<BatchScope>()
        .typ::<BatchAction>()
        .typ::<BatchRequest>()
        .typ::<BatchOutcome>()
        .typ::<BatchPreview>()
        .typ::<BatchReport>()
        .typ::<GuardOperationPhase>()
        .typ::<GuardOperationProgress>()
        .typ::<LifecycleMigrationChoice>()
        .typ::<AuditSourceSupport>()
        .typ::<AuditVerdict>()
        .typ::<AuditNotice>()
        .typ::<ParentDispatchAudit>()
        .typ::<TurnAudit>()
        .typ::<AgentAudit>()
        .typ::<SubagentAuditResult>()
        .typ::<ManagedRoleSnapshot>()
        .typ::<OutboundObservation>()
        .typ::<OperationAuditPhase>()
        .typ::<OperationAuditResult>()
        .typ::<OperationAuditRecord>()
        .typ::<DiagnosticCode>()
        .typ::<DiagnosticSeverity>()
        .typ::<DiagnosticParams>()
        .typ::<ValidationDiagnostic>()
        .typ::<RoleSaveInput>()
        .typ::<RoleCopyInput>()
        .typ::<RoleReorderInput>()
        .typ::<RoleSummaryDto>()
        .typ::<RoleDetailDto>()
        .typ::<RoleListDto>()
        .typ::<RoleDiscoveryDiagnosticDto>()
        .typ::<RoleDiscoveryItemDto>()
        .typ::<RoleDiscoveryDto>()
        .typ::<RoleDeleteReport>()
        .typ::<RoleMigrationResolveInput>()
        .typ::<RoleMigrationDto>()
        .typ::<CapabilityModelDto>()
        .typ::<CapabilitySnapshotDto>()
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
        runtime_state: super::view::GuardRuntimeState::Suspended,
        lifecycle: super::lifecycle::LifecycleSummary::Disabled,
        health: super::lifecycle::HealthStatus::Healthy,
        actions: super::view::ActionStatesView {
            apply: super::view::ActionEligibilityView {
                enabled: true,
                reason: None,
                affected_members: 1,
                affected_files: 1,
            },
            lock: super::view::ActionEligibilityView {
                enabled: true,
                reason: None,
                affected_members: 1,
                affected_files: 1,
            },
            unlock: super::view::ActionEligibilityView {
                enabled: true,
                reason: None,
                affected_members: 0,
                affected_files: 0,
            },
            disable: super::view::ActionEligibilityView {
                enabled: true,
                reason: None,
                affected_members: 0,
                affected_files: 0,
            },
        },
        affected_members: 1,
        affected_files: 1,
        files: vec![super::view::PhysicalFileView {
            id: "config".to_string(),
            name: "Codex config".to_string(),
            file: "config.toml".to_string(),
            format: GuardFileFormat::Toml,
            builtin: true,
            detection: None,
            health: super::lifecycle::HealthStatus::Healthy,
            diagnostics: Vec::new(),
        }],
        recovery: GuardRecoveryStatus {
            blocked: false,
            code: None,
        },
        pending_format_migrations: Vec::new(),
        groups: vec![super::view::GroupView {
            id: "config".to_string(),
            name: "Codex config".to_string(),
            file: "config.toml".to_string(),
            format: GuardFileFormat::Toml,
            builtin: true,
            lifecycle: super::lifecycle::LifecycleSummary::Disabled,
            health: super::lifecycle::HealthStatus::Healthy,
            actions: super::view::ActionStatesView {
                apply: super::view::ActionEligibilityView {
                    enabled: true,
                    reason: None,
                    affected_members: 1,
                    affected_files: 1,
                },
                lock: super::view::ActionEligibilityView {
                    enabled: true,
                    reason: None,
                    affected_members: 1,
                    affected_files: 1,
                },
                unlock: super::view::ActionEligibilityView {
                    enabled: true,
                    reason: None,
                    affected_members: 0,
                    affected_files: 0,
                },
                disable: super::view::ActionEligibilityView {
                    enabled: true,
                    reason: None,
                    affected_members: 0,
                    affected_files: 0,
                },
            },
            affected_members: 1,
            affected_files: 1,
            error: None,
            diagnostics: Vec::new(),
            params: vec![super::view::ParamView {
                id: "demo".to_string(),
                group_id: "uncategorized".to_string(),
                label: "Demo".to_string(),
                description: String::new(),
                apply_mode: "toml_key".to_string(),
                value_type: "bool".to_string(),
                path: "features.demo".to_string(),
                default: serde_json::json!(false),
                value: serde_json::json!(false),
                applied: false,
                locked: false,
                lifecycle: super::lifecycle::ParameterLifecycle::Disabled,
                health: super::lifecycle::HealthStatus::Healthy,
                actions: super::view::ActionStatesView {
                    apply: super::view::ActionEligibilityView {
                        enabled: true,
                        reason: None,
                        affected_members: 1,
                        affected_files: 1,
                    },
                    lock: super::view::ActionEligibilityView {
                        enabled: true,
                        reason: None,
                        affected_members: 1,
                        affected_files: 1,
                    },
                    unlock: super::view::ActionEligibilityView {
                        enabled: true,
                        reason: None,
                        affected_members: 0,
                        affected_files: 0,
                    },
                    disable: super::view::ActionEligibilityView {
                        enabled: true,
                        reason: None,
                        affected_members: 0,
                        affected_files: 0,
                    },
                },
                affected_members: 1,
                affected_files: 1,
                actual: None,
                status: "match".to_string(),
                error: None,
                diagnostics: Vec::new(),
                last_checked: None,
                last_restored: None,
                custom: false,
            }],
            files: vec![super::view::GroupFileRef {
                file: "config.toml".to_string(),
                format: GuardFileFormat::Toml,
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
