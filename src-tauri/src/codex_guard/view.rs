//! 视图组装：把 schema + 托管状态 + 文件实况拼成给前端的 GuardView

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::config::ConfigStore;

use super::batch::{evaluate_eligibility, plan_member_action, BatchAction, EligibilityContext};
use super::engine::{check_many, expected_of};
use super::files::load_files;
use super::lifecycle::{
    aggregate_health, aggregate_lifecycle, HealthStatus, LifecycleSummary, ParameterLifecycle,
};
use super::model::{DiagnosticCode, ValidationDiagnostic};
use super::ownership::validate_ownership;
use super::schema::load_schema;
use super::{
    canonical_group_id, AppPaths, GuardFileFormat, GuardParam, GuardRecoveryStatus,
    PendingFormatMigration,
};

#[derive(Debug, Clone, Copy, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum GuardRuntimeState {
    Running,
    Suspended,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ActionEligibilityView {
    pub enabled: bool,
    pub reason: Option<String>,
    pub affected_members: u32,
    pub affected_files: u32,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ActionStatesView {
    pub apply: ActionEligibilityView,
    pub lock: ActionEligibilityView,
    pub unlock: ActionEligibilityView,
    pub disable: ActionEligibilityView,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalFileView {
    pub id: String,
    pub name: String,
    pub file: String,
    pub format: GuardFileFormat,
    pub builtin: bool,
    pub detection: Option<super::DetectRecord>,
    pub health: HealthStatus,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ParamView {
    pub id: String,
    pub group_id: String,
    pub label: String,
    pub description: String,
    pub apply_mode: String,
    pub value_type: String,
    pub path: String,
    pub default: serde_json::Value,
    pub value: serde_json::Value,
    pub applied: bool,
    pub locked: bool,
    pub lifecycle: ParameterLifecycle,
    pub health: HealthStatus,
    pub actions: ActionStatesView,
    pub affected_members: u32,
    pub affected_files: u32,
    pub actual: Option<String>,
    /// match | drift | missing | error
    pub status: String,
    pub error: Option<String>,
    pub diagnostics: Vec<ValidationDiagnostic>,
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
    pub format: GuardFileFormat,
    pub builtin: bool,
    pub lifecycle: LifecycleSummary,
    pub health: HealthStatus,
    pub actions: ActionStatesView,
    pub affected_members: u32,
    pub affected_files: u32,
    pub error: Option<String>,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub params: Vec<ParamView>,
    /// A logical group may span multiple physical files. `file` remains the first
    /// reference for old clients; new clients should use this complete list.
    pub files: Vec<GroupFileRef>,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GroupFileRef {
    pub file: String,
    pub format: GuardFileFormat,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GuardView {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u16,
    pub enabled: bool,
    pub runtime_state: GuardRuntimeState,
    pub lifecycle: LifecycleSummary,
    pub health: HealthStatus,
    pub actions: ActionStatesView,
    pub affected_members: u32,
    pub affected_files: u32,
    pub files: Vec<PhysicalFileView>,
    pub recovery: GuardRecoveryStatus,
    pub groups: Vec<GroupView>,
    pub pending_format_migrations: Vec<PendingFormatMigration>,
}

#[derive(Debug, Clone)]
struct MemberSnapshot {
    param: GuardParam,
    group_id: String,
    file: String,
    format: GuardFileFormat,
    expected: serde_json::Value,
    lifecycle: ParameterLifecycle,
    health: HealthStatus,
    status: String,
    actual: Option<String>,
    error: Option<String>,
    diagnostics: Vec<ValidationDiagnostic>,
    last_checked: Option<u64>,
    last_restored: Option<u64>,
    pending_migration: bool,
}

fn action_eligibility(action: BatchAction, members: &[&MemberSnapshot]) -> ActionEligibilityView {
    if members.is_empty() {
        return ActionEligibilityView {
            enabled: false,
            reason: Some("empty_scope".to_string()),
            affected_members: 0,
            affected_files: 0,
        };
    }

    let mut enabled = true;
    let mut reason = None;
    let mut affected_members = 0;
    let mut affected_files = BTreeSet::new();
    for member in members {
        if member.pending_migration {
            enabled = false;
            reason.get_or_insert_with(|| "lifecycle_migration_pending".to_string());
        } else {
            let eligibility = evaluate_eligibility(
                action,
                EligibilityContext::new(member.lifecycle, member.health),
            );
            if !eligibility.eligible {
                enabled = false;
                reason.get_or_insert_with(|| {
                    eligibility
                        .reason
                        .map(|reason| reason.as_str().to_string())
                        .unwrap_or_else(|| "state_unavailable".to_string())
                });
            }
        }

        let plan = plan_member_action(action, member.lifecycle, member.health);
        if plan.changed {
            affected_members += 1;
            if plan.writes_file {
                affected_files.insert(member.file.clone());
            }
        }
    }

    ActionEligibilityView {
        enabled,
        reason,
        affected_members,
        affected_files: affected_files.len() as u32,
    }
}

fn action_states(members: &[&MemberSnapshot]) -> ActionStatesView {
    ActionStatesView {
        apply: action_eligibility(BatchAction::Apply, members),
        lock: action_eligibility(BatchAction::Lock, members),
        unlock: action_eligibility(BatchAction::Unlock, members),
        disable: action_eligibility(BatchAction::Disable, members),
    }
}

fn param_view(member: &MemberSnapshot) -> ParamView {
    let members = [member];
    let actions = action_states(&members);
    let affected_files =
        u32::from(actions.apply.affected_files > 0 || actions.lock.affected_files > 0);
    ParamView {
        id: member.param.id.clone(),
        group_id: member.group_id.clone(),
        label: member.param.localized_label().to_string(),
        description: member.param.localized_description().to_string(),
        apply_mode: member.param.apply_mode.clone(),
        value_type: member.param.value_type.clone(),
        path: member.param.path.clone(),
        default: member.param.default.clone(),
        value: member.expected.clone(),
        applied: member.lifecycle.is_enabled(),
        locked: member.lifecycle.is_locked(),
        lifecycle: member.lifecycle,
        health: member.health,
        affected_members: 1,
        affected_files,
        actions,
        actual: member.actual.clone(),
        status: member.status.clone(),
        error: member.error.clone(),
        diagnostics: member.diagnostics.clone(),
        last_checked: member.last_checked,
        last_restored: member.last_restored,
        custom: member.param.custom,
    }
}

fn canonical_config_group_id(id: &str) -> &str {
    match id {
        "subagent-operations" => "subagent-optimization",
        other => other,
    }
}

fn group_definition<'a>(
    groups: &'a [super::GuardGroup],
    id: &str,
) -> Option<&'a super::GuardGroup> {
    groups
        .iter()
        .find(|group| canonical_config_group_id(&group.id) == id)
}

pub fn build_view(
    store: &ConfigStore,
    paths: &AppPaths,
    recovery_status: GuardRecoveryStatus,
) -> Result<GuardView, String> {
    let cfg = store.load_launcher()?;
    let schema = load_schema(store)?;
    let files = load_files(store)?;
    validate_ownership(paths, &files, &schema).map_err(|error| error.to_string())?;

    let mut checks_by_id = BTreeMap::new();
    for file in &files {
        let file_params = schema
            .iter()
            .filter(|param| param.effective_file_id() == file.id)
            .collect::<Vec<_>>();
        if file_params.is_empty() {
            continue;
        }
        let expected_values = file_params
            .iter()
            .map(|param| expected_of(param, cfg.codex_guard.params.get(&param.id)))
            .collect::<Vec<_>>();
        let check_targets = file_params
            .iter()
            .zip(expected_values.iter())
            .map(|(param, expected)| (*param, expected))
            .collect::<Vec<_>>();
        for (param, check) in
            file_params
                .iter()
                .zip(check_many(paths, &file.file, file.format, &check_targets))
        {
            checks_by_id.insert(
                param.id.clone(),
                (
                    file.file.clone(),
                    file.format,
                    check.status,
                    check.actual,
                    check.error,
                ),
            );
        }
    }

    let mut members = Vec::new();
    for param in &schema {
        let Some((file, format, status, actual, error)) = checks_by_id.remove(&param.id) else {
            continue;
        };
        let state = cfg.codex_guard.params.get(&param.id);
        let pending_migration = cfg
            .codex_guard
            .pending_lifecycle_migrations
            .contains_key(&param.id);
        let lifecycle = state
            .map(|state| state.lifecycle())
            .unwrap_or(ParameterLifecycle::Disabled);
        let health = if pending_migration {
            HealthStatus::Invalid
        } else {
            match status.as_str() {
                "match" => HealthStatus::Healthy,
                "drift" | "missing" => HealthStatus::Drifted,
                _ => HealthStatus::Error,
            }
        };
        let diagnostics = if error.is_some() {
            vec![ValidationDiagnostic::new(
                &param.id,
                Some(&file),
                DiagnosticCode::PlanConflict,
                None,
                None,
            )]
        } else {
            Vec::new()
        };
        members.push(MemberSnapshot {
            param: param.clone(),
            group_id: canonical_group_id(&param.id, param.group_id.as_deref(), param.custom),
            file,
            format,
            expected: expected_of(param, state),
            lifecycle,
            health,
            status,
            actual,
            error,
            diagnostics,
            last_checked: state.and_then(|state| state.last_checked),
            last_restored: state.and_then(|state| state.last_restored),
            pending_migration,
        });
    }

    let mut group_ids = members
        .iter()
        .map(|member| member.group_id.clone())
        .collect::<BTreeSet<_>>();
    for group in &cfg.codex_guard.groups {
        group_ids.insert(canonical_config_group_id(&group.id).to_string());
    }
    let mut ordered_group_ids = group_ids.into_iter().collect::<Vec<_>>();
    ordered_group_ids.sort_by_key(|id| {
        group_definition(&cfg.codex_guard.groups, id)
            .map(|group| (if group.builtin { 0 } else { 1 }, group.order, id.clone()))
            .unwrap_or((2, u32::MAX, id.clone()))
    });

    let mut groups = Vec::new();
    for group_id in ordered_group_ids {
        let group_members = members
            .iter()
            .filter(|member| member.group_id == group_id)
            .collect::<Vec<_>>();
        let mut group_files = BTreeMap::new();
        let mut group_error = None;
        let mut group_diagnostics = Vec::new();
        let mut params = Vec::new();
        for member in &group_members {
            group_files.insert(member.file.clone(), member.format);
            if group_error.is_none() && member.error.is_some() {
                group_error = member.error.clone();
            }
            group_diagnostics.extend(member.diagnostics.clone());
            params.push(param_view(member));
        }
        let action_members = group_members.clone();
        let actions = action_states(&action_members);
        let files = group_files
            .into_iter()
            .map(|(file, format)| GroupFileRef { file, format })
            .collect::<Vec<_>>();
        let first = files.first();
        let group = group_definition(&cfg.codex_guard.groups, &group_id);
        groups.push(GroupView {
            id: group_id.clone(),
            name: group.map(|group| group.name.clone()).unwrap_or_else(|| {
                if group_id == "uncategorized" {
                    "Uncategorized".to_string()
                } else {
                    group_id.clone()
                }
            }),
            file: first.map(|file| file.file.clone()).unwrap_or_default(),
            format: first
                .map(|file| file.format)
                .unwrap_or(GuardFileFormat::PlainText),
            builtin: group.map(|group| group.builtin).unwrap_or(false),
            lifecycle: aggregate_lifecycle(group_members.iter().map(|member| member.lifecycle)),
            health: aggregate_health(group_members.iter().map(|member| member.health)),
            affected_members: group_members.len() as u32,
            affected_files: files.len() as u32,
            actions,
            error: group_error,
            diagnostics: group_diagnostics,
            params,
            files,
        });
    }

    let mut physical_files = Vec::new();
    for file in files {
        let file_members = members
            .iter()
            .filter(|member| member.file == file.file)
            .collect::<Vec<_>>();
        let diagnostics = file_members
            .iter()
            .flat_map(|member| member.diagnostics.clone())
            .collect::<Vec<_>>();
        physical_files.push(PhysicalFileView {
            id: file.id,
            name: file.name,
            file: file.file,
            format: file.format,
            builtin: file.builtin,
            detection: file.detection,
            health: aggregate_health(file_members.iter().map(|member| member.health)),
            diagnostics,
        });
    }

    let action_members = members.iter().collect::<Vec<_>>();
    let pending_migrations = !cfg.codex_guard.pending_lifecycle_migrations.is_empty();
    let mut pending_format_migrations = cfg
        .codex_guard
        .pending_format_migrations
        .values()
        .cloned()
        .collect::<Vec<_>>();
    pending_format_migrations.sort_by(|left, right| left.id.cmp(&right.id));
    let recovery = if recovery_status.blocked {
        recovery_status
    } else if !pending_format_migrations.is_empty() {
        GuardRecoveryStatus {
            blocked: true,
            code: Some("format_migration_pending".to_string()),
        }
    } else if pending_migrations {
        GuardRecoveryStatus {
            blocked: true,
            code: Some("lifecycle_migration_pending".to_string()),
        }
    } else {
        GuardRecoveryStatus {
            blocked: false,
            code: None,
        }
    };
    Ok(GuardView {
        schema_version: super::contracts::GUARD_CONTRACT_SCHEMA_VERSION,
        enabled: cfg.codex_guard.enabled,
        runtime_state: if cfg.codex_guard.enabled {
            GuardRuntimeState::Running
        } else {
            GuardRuntimeState::Suspended
        },
        lifecycle: aggregate_lifecycle(action_members.iter().map(|member| member.lifecycle)),
        health: aggregate_health(action_members.iter().map(|member| member.health)),
        actions: action_states(&action_members),
        affected_members: members.len() as u32,
        affected_files: physical_files.len() as u32,
        files: physical_files,
        recovery,
        pending_format_migrations,
        groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(pending_migration: bool) -> MemberSnapshot {
        MemberSnapshot {
            param: GuardParam {
                id: "features.multi_agent_v2.enabled".to_string(),
                label: "Multi-agent".to_string(),
                label_en: String::new(),
                description: String::new(),
                description_en: String::new(),
                file: "config.toml".to_string(),
                file_id: "builtin.config-toml".to_string(),
                group_id: Some("subagent-operations".to_string()),
                apply_mode: "toml_key".to_string(),
                path: "features.multi_agent_v2.enabled".to_string(),
                value_type: "bool".to_string(),
                default: serde_json::json!(false),
                default_en: serde_json::Value::Null,
                custom: false,
            },
            group_id: "subagent-optimization".to_string(),
            file: "config.toml".to_string(),
            format: GuardFileFormat::Toml,
            expected: serde_json::json!(false),
            lifecycle: ParameterLifecycle::Disabled,
            health: if pending_migration {
                HealthStatus::Invalid
            } else {
                HealthStatus::Healthy
            },
            status: "match".to_string(),
            actual: None,
            error: None,
            diagnostics: Vec::new(),
            last_checked: None,
            last_restored: None,
            pending_migration,
        }
    }

    #[test]
    fn pending_lifecycle_migration_disables_every_action() {
        let member = member(true);
        let members = [&member];
        let actions = action_states(&members);

        for action in [actions.apply, actions.lock, actions.unlock, actions.disable] {
            assert!(!action.enabled);
            assert_eq!(
                action.reason.as_deref(),
                Some("lifecycle_migration_pending")
            );
        }
    }

    #[test]
    fn legacy_subagent_group_id_is_canonicalized() {
        assert_eq!(
            canonical_group_id(
                "features.multi_agent_v2.enabled",
                Some("subagent-operations"),
                false,
            ),
            "subagent-optimization"
        );
        assert_eq!(
            canonical_config_group_id("subagent-operations"),
            "subagent-optimization"
        );
    }
}
