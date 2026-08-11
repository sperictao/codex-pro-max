//! Tauri 命令：前端调用的全部入口（参数管理 / 自定义参数 / 看守文件管理 / 路径检测）

use crate::config::LauncherConfig;
use crate::i18n::{tr, trf};
use crate::AppState;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use super::atomic_store::PlatformAtomicFileWriter;
use super::audit::OperationAuditResult;
use super::batch::{
    evaluate_eligibility, plan_member_action, BatchAction, BatchEligibilityReason, BatchMember,
    BatchOutcome, BatchPreview, BatchReport, BatchRequest, BatchScope, GuardOperationPhase,
    GuardOperationProgress,
};
use super::engine::{
    execute_transaction_batch, expected_of, prepare_file_plan, recover_pending_transactions,
    ManagedMember, PlannedFileWrite, TransactionWrite,
};
use super::files::{builtin_files, detect_file_path, find_file, load_files, update_files};
use super::journal::JournalParticipant;
use super::lifecycle::{HealthStatus, ParameterLifecycle};
use super::operation_audit::OperationAuditGuard;
use super::ownership::{normalize_relative_path, validate_ownership, validate_target_path};
use super::roles::{role_relative_path, RoleHealth, RoleLifecycle};
use super::roles_store::{
    load_role_states, read_role_file, render_role_directory_content, ManagedRoleState,
    ROLE_DIRECTORY_FILE, ROLE_DIRECTORY_MEMBER_ID,
};
use super::schema::{ensure_schema_file, load_schema, schema_file_path, update_disk_schema};
use super::transaction::is_recovery_blocking_error;
use super::validate::{
    normalize_custom_id, validate_guard_file, validate_param_fields, validate_param_for_file,
};
use super::view::{build_view, GuardView};
use super::{
    canonical_group_id, now_secs, DetectRecord, GuardFile, GuardFileFormat, GuardGroup, GuardParam,
    GuardParamState, GuardRecoveryStatus, LifecycleMigrationChoice,
};

#[derive(Debug, Clone)]
struct BatchParamContext {
    param: GuardParam,
    format: GuardFileFormat,
    expected: serde_json::Value,
    lifecycle: ParameterLifecycle,
    health: HealthStatus,
}

#[derive(Debug, Clone)]
struct BatchRoleContext {
    state: ManagedRoleState,
    health: RoleHealth,
    actual_file_present: bool,
}

impl BatchRoleContext {
    fn id(&self) -> &str {
        self.state.id().as_str()
    }

    fn lifecycle(&self) -> RoleLifecycle {
        self.state.lifecycle
    }
}

#[derive(Debug, Default)]
struct SelectedBatch<'a> {
    params: Vec<&'a BatchParamContext>,
    roles: Vec<&'a BatchRoleContext>,
}

const ROLE_GROUP_ID: &str = "subagent-optimization";
const ROLE_FILE_ID_PREFIX: &str = "role:";

fn role_lifecycle_as_parameter(lifecycle: RoleLifecycle) -> ParameterLifecycle {
    match lifecycle {
        RoleLifecycle::Disabled => ParameterLifecycle::Disabled,
        RoleLifecycle::Applied => ParameterLifecycle::Applied,
        RoleLifecycle::Locked => ParameterLifecycle::Locked,
    }
}

fn role_health_as_parameter(health: RoleHealth) -> HealthStatus {
    match health {
        RoleHealth::Healthy => HealthStatus::Healthy,
        RoleHealth::Drifted => HealthStatus::Drifted,
        RoleHealth::Invalid => HealthStatus::Invalid,
        RoleHealth::Unsupported => HealthStatus::Unsupported,
        RoleHealth::Error => HealthStatus::Error,
    }
}

fn role_member_id(role_id: &str) -> String {
    format!("{ROLE_FILE_ID_PREFIX}{role_id}")
}

fn scope_includes_roles(scope: &BatchScope) -> bool {
    match scope {
        BatchScope::All | BatchScope::Role { .. } => true,
        BatchScope::Group { group_id } => group_id == ROLE_GROUP_ID,
        BatchScope::Parameter { .. } => false,
    }
}

fn load_batch_roles(
    paths: &super::AppPaths,
    store: &crate::config::ConfigStore,
    config: &LauncherConfig,
    action: BatchAction,
) -> Result<Vec<BatchRoleContext>, String> {
    let states = load_role_states(store)?;
    let capabilities = if action.writes_target_file() {
        super::capability::current_capability(
            &config.codex_app_path,
            now_secs().saturating_mul(1_000),
        )
        .ok()
    } else {
        None
    };
    states
        .into_iter()
        .map(|state| {
            let actual = read_role_file(paths, state.id())?;
            let actual_file_present = actual.is_some();
            let managed = state
                .clone()
                .into_managed(actual_file_present, capabilities.as_ref())
                .map_err(|error| error.to_string())?;
            let health = if matches!(managed.health, RoleHealth::Healthy) {
                let mut expected = state.record.expected_toml.trim().to_string();
                expected.push('\n');
                if actual.as_deref() != Some(expected.as_bytes()) {
                    RoleHealth::Drifted
                } else {
                    managed.health
                }
            } else {
                managed.health
            };
            Ok(BatchRoleContext {
                state,
                health,
                actual_file_present,
            })
        })
        .collect()
}

fn load_batch_context_from_config(
    paths: &super::AppPaths,
    store: &crate::config::ConfigStore,
    config: &LauncherConfig,
    migration_parameter_id: Option<&str>,
) -> Result<Vec<BatchParamContext>, String> {
    let schema = load_schema(store)?;
    let files = load_files(store)?;
    validate_configuration(paths, &files, &schema)?;
    let mut result = Vec::new();
    for file in files {
        let file_params = schema
            .iter()
            .filter(|param| param.effective_file_id() == file.id)
            .collect::<Vec<_>>();
        if file_params.is_empty() {
            continue;
        }
        let expected = file_params
            .iter()
            .map(|param| expected_of(param, config.codex_guard.params.get(&param.id)))
            .collect::<Vec<_>>();
        let targets = file_params
            .iter()
            .zip(expected.iter())
            .map(|(param, value)| (*param, value))
            .collect::<Vec<_>>();
        let checks = super::engine::check_many(paths, &file.file, file.format, &targets);
        for ((param, expected), check) in file_params.into_iter().zip(expected).zip(checks) {
            let state = config.codex_guard.params.get(&param.id);
            let migration_pending = config
                .codex_guard
                .pending_lifecycle_migrations
                .contains_key(&param.id)
                && migration_parameter_id != Some(param.id.as_str());
            let lifecycle = state
                .map(GuardParamState::lifecycle)
                .unwrap_or(ParameterLifecycle::Disabled);
            let health = if migration_pending {
                HealthStatus::Invalid
            } else {
                match check.status.as_str() {
                    "match" => HealthStatus::Healthy,
                    "drift" | "missing" => HealthStatus::Drifted,
                    _ => HealthStatus::Error,
                }
            };
            result.push(BatchParamContext {
                param: param.clone(),
                format: file.format,
                expected,
                lifecycle,
                health,
            });
        }
    }
    result.sort_by(|left, right| left.param.id.cmp(&right.param.id));
    Ok(result)
}

fn select_batch_context<'a>(
    scope: &BatchScope,
    contexts: &'a [BatchParamContext],
    roles: &'a [BatchRoleContext],
) -> Result<SelectedBatch<'a>, String> {
    let param_members = contexts
        .iter()
        .map(|context| {
            BatchMember::new(&context.param.id, context.lifecycle, context.health).in_group(
                canonical_group_id(
                    &context.param.id,
                    context.param.group_id.as_deref(),
                    context.param.custom,
                ),
            )
        })
        .collect::<Vec<_>>();
    let role_members = roles
        .iter()
        .map(|context| {
            BatchMember::new(
                context.id(),
                role_lifecycle_as_parameter(context.lifecycle()),
                role_health_as_parameter(context.health),
            )
            .in_group(ROLE_GROUP_ID)
            .for_role(context.id())
        })
        .collect::<Vec<_>>();

    let mut selected = SelectedBatch::default();
    match scope {
        BatchScope::Role { .. } => {
            let role_ids = super::batch::members_in_scope(scope, &role_members)
                .into_iter()
                .map(|member| member.id.as_str())
                .collect::<BTreeSet<_>>();
            selected.roles = roles
                .iter()
                .filter(|role| role_ids.contains(role.id()))
                .collect();
            if selected.roles.is_empty() {
                return Err("batch_scope_empty".to_string());
            }
        }
        _ => {
            let param_ids = super::batch::members_in_scope(scope, &param_members)
                .into_iter()
                .map(|member| member.id.as_str())
                .collect::<BTreeSet<_>>();
            selected.params = contexts
                .iter()
                .filter(|context| param_ids.contains(context.param.id.as_str()))
                .collect();

            let role_ids = super::batch::members_in_scope(scope, &role_members)
                .into_iter()
                .map(|member| member.id.as_str())
                .collect::<BTreeSet<_>>();
            selected.roles = roles
                .iter()
                .filter(|role| role_ids.contains(role.id()))
                .collect();
        }
    }
    if selected.params.is_empty() && selected.roles.is_empty() {
        return Err("batch_scope_empty".to_string());
    }
    Ok(selected)
}

fn insert_plan_member(
    members_by_file: &mut BTreeMap<String, (GuardFileFormat, Vec<ManagedMember>)>,
    file: String,
    format: GuardFileFormat,
    member: ManagedMember,
) -> Result<(), String> {
    let entry = members_by_file
        .entry(file)
        .or_insert_with(|| (format, Vec::new()));
    if entry.0 != format {
        return Err("batch_file_format_conflict".to_string());
    }
    entry.1.push(member);
    Ok(())
}

type PreparedBatchPlan = (PathBuf, Option<Vec<u8>>, PlannedFileWrite);

fn prepare_batch_plans(
    paths: &super::AppPaths,
    selected: &SelectedBatch<'_>,
    all_roles: &[BatchRoleContext],
    action: BatchAction,
) -> Result<Vec<PreparedBatchPlan>, String> {
    let mut members_by_file: BTreeMap<String, (GuardFileFormat, Vec<ManagedMember>)> =
        BTreeMap::new();
    for context in &selected.params {
        let action_plan = plan_member_action(action, context.lifecycle, context.health);
        if !action_plan.writes_file {
            continue;
        }
        insert_plan_member(
            &mut members_by_file,
            context.param.file.clone(),
            context.format,
            ManagedMember {
                id: context.param.id.clone(),
                apply_mode: context.param.apply_mode.clone(),
                path: context.param.path.clone(),
                value_type: context.param.value_type.clone(),
                expected: context.expected.clone(),
            },
        )?;
    }

    for role in &selected.roles {
        let action_plan = plan_member_action(
            action,
            role_lifecycle_as_parameter(role.lifecycle()),
            role_health_as_parameter(role.health),
        );
        if !action_plan.writes_file {
            continue;
        }
        insert_plan_member(
            &mut members_by_file,
            role_relative_path(role.state.id()),
            GuardFileFormat::Toml,
            ManagedMember {
                id: role_member_id(role.id()),
                apply_mode: "file_overwrite".to_string(),
                path: String::new(),
                value_type: "text".to_string(),
                expected: serde_json::Value::String(role.state.record.expected_toml.clone()),
            },
        )?;
    }

    if action.writes_target_file() && !selected.roles.is_empty() {
        let mut present_ids = all_roles
            .iter()
            .filter(|role| role.actual_file_present)
            .map(|role| role.id().to_string())
            .collect::<std::collections::HashSet<_>>();
        for role in &selected.roles {
            let role_plan = plan_member_action(
                action,
                role_lifecycle_as_parameter(role.lifecycle()),
                role_health_as_parameter(role.health),
            );
            if role_plan.writes_file {
                present_ids.insert(role.id().to_string());
            }
        }
        let states = all_roles
            .iter()
            .map(|role| role.state.clone())
            .collect::<Vec<_>>();
        let expected = render_role_directory_content(&states, &present_ids)?;
        insert_plan_member(
            &mut members_by_file,
            ROLE_DIRECTORY_FILE.to_string(),
            GuardFileFormat::Markdown,
            ManagedMember {
                id: ROLE_DIRECTORY_MEMBER_ID.to_string(),
                apply_mode: "markdown_block".to_string(),
                path: String::new(),
                value_type: "text".to_string(),
                expected: serde_json::Value::String(expected),
            },
        )?;
    }

    members_by_file
        .into_iter()
        .map(|(file, (format, members))| prepare_file_plan(paths, &file, format, &members))
        .collect()
}

fn eligibility_reason(
    context: &BatchParamContext,
    action: BatchAction,
) -> Option<BatchEligibilityReason> {
    let eligibility = evaluate_eligibility(
        action,
        super::batch::EligibilityContext::new(context.lifecycle, context.health),
    );
    eligibility.reason
}

fn find_param(schema: &[GuardParam], id: &str) -> Result<GuardParam, String> {
    schema.iter().find(|p| p.id == id).cloned().ok_or_else(|| {
        trf(
            "Parameter not found in schema: {id}",
            &[("id", id.to_string())],
        )
    })
}

fn validate_configuration(
    paths: &super::AppPaths,
    files: &[GuardFile],
    schema: &[GuardParam],
) -> Result<(), String> {
    validate_ownership(paths, files, schema).map_err(|error| error.to_string())
}

fn config_revision(config: &LauncherConfig) -> Result<String, String> {
    // Polling updates these timestamps as runtime evidence. They must not make a
    // previously validated batch plan stale; lifecycle, values, groups, roles,
    // and all other persisted configuration still participate in the revision.
    let mut semantic_config = config.clone();
    for state in semantic_config.codex_guard.params.values_mut() {
        state.last_checked = None;
        state.last_restored = None;
    }
    let value = serde_json::to_value(semantic_config).map_err(|error| error.to_string())?;
    let value = canonicalize_revision_json(value);
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn validate_migration_override(
    config: &LauncherConfig,
    migration_parameter_id: Option<&str>,
) -> Result<(), String> {
    if let Some(parameter_id) = migration_parameter_id {
        if !config
            .codex_guard
            .pending_lifecycle_migrations
            .contains_key(parameter_id)
        {
            return Err("lifecycle_migration_not_pending".to_string());
        }
    }
    Ok(())
}

fn reject_pending_format_migration(config: &LauncherConfig) -> Result<(), String> {
    if config.codex_guard.pending_format_migrations.is_empty() {
        Ok(())
    } else {
        Err("format_migration_pending".to_string())
    }
}

fn canonicalize_revision_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let sorted = object
                .into_iter()
                .map(|(key, value)| (key, canonicalize_revision_json(value)))
                .collect::<std::collections::BTreeMap<_, _>>();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize_revision_json).collect())
        }
        value => value,
    }
}

fn audit_scope(scope: &BatchScope) -> String {
    match scope {
        BatchScope::All => "all".to_string(),
        BatchScope::Group { group_id } => format!("group:{group_id}"),
        BatchScope::Parameter { parameter_id } => format!("parameter:{parameter_id}"),
        BatchScope::Role { role_id } => format!("role:{role_id}"),
    }
}

fn emit_batch_progress(
    app: &AppHandle,
    batch_id: &str,
    phase: GuardOperationPhase,
    completed: u32,
) {
    let event = GuardOperationProgress {
        batch_id: batch_id.to_string(),
        phase,
        completed,
        total: 6,
    };
    // A progress notification is advisory and must never make a committed transaction fail.
    // The payload is the typed, path/value-free contract; the legacy event name is identical.
    if let Err(error) = event.emit(app) {
        log::debug!("guard operation progress emit failed: {error}");
    }
}

fn stable_batch_error_code(error: &str) -> String {
    if error.starts_with("guard transaction failed: ") {
        "guard_transaction_failed".to_string()
    } else if error == "recovery_blocked" {
        "recovery_blocked".to_string()
    } else if error.starts_with("batch_") {
        error.to_string()
    } else {
        "batch_failed".to_string()
    }
}

fn stable_operation_error_code(error: &str) -> String {
    let candidate = error
        .strip_prefix("guard transaction failed: ")
        .unwrap_or(error);
    if !candidate.is_empty()
        && candidate.len() <= 64
        && candidate
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        candidate.to_string()
    } else {
        "operation_failed".to_string()
    }
}

fn record_command_error(
    audit: &mut OperationAuditGuard<'_>,
    error: &str,
    changed: u32,
    unchanged: u32,
    files: u32,
) {
    let code = stable_operation_error_code(error);
    if error == "guard_busy" {
        audit.busy(code, changed, unchanged, files);
    } else if is_recovery_blocking_error(error) {
        audit.critical(code, changed, unchanged, files);
    } else if error.starts_with("batch_")
        || error == "guard_contract_version_unsupported"
        || error.ends_with("_ineligible")
        || error.ends_with("_invalid")
        || error.ends_with("_not_found")
        || error == "group_name_duplicate"
        || error == "group_not_empty"
        || error == "lifecycle_migration_not_pending"
        || error == "role_not_managed"
        || error == "builtin_group_immutable"
        || error == "builtin_parameter_immutable"
    {
        audit.rejected(code, changed, unchanged, files);
    } else if error.starts_with("guard transaction failed: ") {
        audit.rolled_back(code, changed, unchanged, files);
    } else {
        audit.failure(code, changed, unchanged, files);
    }
}

fn finish_command_result<T>(
    audit: &mut OperationAuditGuard<'_>,
    result: Result<T, String>,
) -> Result<T, String> {
    if let Err(error) = &result {
        record_command_error(audit, error, 0, 0, 0);
    }
    result
}

fn return_guard_transaction_result<T>(
    state: &AppState,
    result: Result<T, String>,
) -> Result<T, String> {
    if result
        .as_ref()
        .is_err_and(|error| is_recovery_blocking_error(error))
    {
        state
            .guard_coordinator
            .mark_recovery_blocked("recovery_failed");
    }
    result
}

#[tauri::command]
#[specta::specta]
pub fn guard_get_view(state: State<'_, AppState>) -> Result<GuardView, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "view:get", None);
    let result = (|| {
        let view = build_view(
            &state.config_store,
            &state.paths,
            state.guard_coordinator.recovery_status(),
        )?;
        audit.success(0, view.affected_members, view.affected_files);
        Ok(view)
    })();
    finish_command_result(&mut audit, result)
}

/// 返回启动恢复是否阻断了 Guard 写入。只暴露稳定 code，不泄漏 journal 细节。
#[tauri::command]
#[specta::specta]
pub fn guard_get_recovery_status(
    state: State<'_, AppState>,
) -> Result<GuardRecoveryStatus, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "recovery:status", None);
    let status = state.guard_coordinator.recovery_status();
    audit.success(0, 0, 0);
    Ok(status)
}

/// 重试未完成事务恢复；成功后按需启动唯一的 Guard 轮询任务。
#[tauri::command]
#[specta::specta]
pub fn guard_retry_recovery(state: State<'_, AppState>) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "recovery:retry", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_write()?;
        // 先恢复未完成事务：它只依赖 journal 与目标文件字节。若先跑迁移并在失败处早退，
        // 「config.json 损坏 + 存在未完成事务」将永远无法恢复。
        let recovered = recover_pending_transactions(&state.paths).map_err(|_| {
            state
                .guard_coordinator
                .mark_recovery_blocked("recovery_failed");
            "recovery_failed".to_string()
        });
        let migrated = state.config_store.migrate_guard_state().map_err(|error| {
            log::error!("codex guard state migration retry failed: {}", error);
            state
                .guard_coordinator
                .mark_recovery_blocked("migration_failed");
            "migration_failed".to_string()
        });
        recovered?;
        migrated?;
        state.guard_coordinator.clear_recovery();
        if state.guard_coordinator.claim_poll_start() {
            tauri::async_runtime::spawn(super::poll_loop(
                state.config_store.clone(),
                state.paths.clone(),
                state.guard_coordinator.clone(),
            ));
        }
        audit.success(0, 0, 0);
        Ok(())
    })();
    finish_command_result(&mut audit, result)
}

fn preview_batch_inner(
    state: &AppState,
    request: &BatchRequest,
    migration_parameter_id: Option<&str>,
) -> Result<BatchPreview, String> {
    let config = state.config_store.load_launcher()?;
    reject_pending_format_migration(&config)?;
    validate_migration_override(&config, migration_parameter_id)?;
    let revision = config_revision(&config)?;
    let contexts = load_batch_context_from_config(
        &state.paths,
        &state.config_store,
        &config,
        migration_parameter_id,
    )?;
    let roles = if scope_includes_roles(&request.scope) {
        load_batch_roles(&state.paths, &state.config_store, &config, request.action)?
    } else {
        Vec::new()
    };
    let selected = select_batch_context(&request.scope, &contexts, &roles)?;
    let mut diagnostics = Vec::new();
    for context in &selected.params {
        if let Some(reason) = eligibility_reason(context, request.action) {
            diagnostics.push(super::model::ValidationDiagnostic::new(
                &context.param.id,
                Some(&context.param.file),
                match reason {
                    BatchEligibilityReason::Unsupported => {
                        super::model::DiagnosticCode::PlanModeIncompatible
                    }
                    BatchEligibilityReason::Invalid
                    | BatchEligibilityReason::Error
                    | BatchEligibilityReason::StateUnavailable
                    | BatchEligibilityReason::RecoveryInProgress
                    | BatchEligibilityReason::OwnershipUncertain => {
                        super::model::DiagnosticCode::PlanConflict
                    }
                },
                None,
                None,
            ));
        }
    }
    for role in &selected.roles {
        let context = super::batch::EligibilityContext::new(
            role_lifecycle_as_parameter(role.lifecycle()),
            role_health_as_parameter(role.health),
        );
        if let Some(reason) = evaluate_eligibility(request.action, context).reason {
            diagnostics.push(super::model::ValidationDiagnostic::new(
                role.id(),
                Some(&role_relative_path(role.state.id())),
                match reason {
                    BatchEligibilityReason::Unsupported => {
                        super::model::DiagnosticCode::PlanModeIncompatible
                    }
                    BatchEligibilityReason::Invalid
                    | BatchEligibilityReason::Error
                    | BatchEligibilityReason::StateUnavailable
                    | BatchEligibilityReason::RecoveryInProgress
                    | BatchEligibilityReason::OwnershipUncertain => {
                        super::model::DiagnosticCode::PlanConflict
                    }
                },
                None,
                None,
            ));
        }
    }
    if diagnostics.is_empty() && request.action.writes_target_file() {
        prepare_batch_plans(&state.paths, &selected, &roles, request.action)?;
    }
    let mut member_ids = selected
        .params
        .iter()
        .map(|context| context.param.id.clone())
        .chain(selected.roles.iter().map(|role| role.id().to_string()))
        .collect::<Vec<_>>();
    member_ids.sort();
    let mut files = selected
        .params
        .iter()
        .map(|context| context.param.file.clone())
        .collect::<BTreeSet<_>>();
    for role in &selected.roles {
        files.insert(role_relative_path(role.state.id()));
    }
    if request.action.writes_target_file() && !selected.roles.is_empty() {
        files.insert(ROLE_DIRECTORY_FILE.to_string());
    }
    let mut changed = 0u32;
    for context in &selected.params {
        if plan_member_action(request.action, context.lifecycle, context.health).changed {
            changed = changed.saturating_add(1);
        }
    }
    for role in &selected.roles {
        if plan_member_action(
            request.action,
            role_lifecycle_as_parameter(role.lifecycle()),
            role_health_as_parameter(role.health),
        )
        .changed
        {
            changed = changed.saturating_add(1);
        }
    }
    let affected_members = member_ids.len() as u32;
    let unchanged = affected_members.saturating_sub(changed);
    let preview_id = super::journal::new_batch_id();
    let preview = BatchPreview {
        schema_version: super::batch::BATCH_CONTRACT_SCHEMA_VERSION,
        preview_id: preview_id.clone(),
        revision: revision.clone(),
        scope: request.scope.clone(),
        action: request.action,
        member_ids,
        affected_members,
        affected_files: files.len() as u32,
        changed,
        unchanged,
        files: files.len() as u32,
        eligible: diagnostics.is_empty(),
        blockers: diagnostics.clone(),
        diagnostics,
    };
    state
        .guard_coordinator
        .remember_preview(preview_id, request.clone(), revision);
    Ok(preview)
}

/// 预检一次全局/组级批量动作。预检只读取文件，不落盘，并在进程内保留五分钟。
#[tauri::command]
#[specta::specta]
pub fn guard_preview_batch(
    state: State<'_, AppState>,
    request: BatchRequest,
) -> Result<BatchPreview, String> {
    let mut audit = OperationAuditGuard::new(
        &state.paths,
        format!("batch:preview:{}", audit_scope(&request.scope)),
        match &request.scope {
            BatchScope::Role { role_id } => Some(role_id.as_str()),
            _ => None,
        },
    );
    let result = (|| {
        if !request.is_supported_version() {
            audit.rejected("guard_contract_version_unsupported", 0, 0, 0);
            return Err("guard_contract_version_unsupported".to_string());
        }
        let _write = match state.guard_coordinator.try_guard_write() {
            Ok(write) => write,
            Err(error) => {
                record_command_error(&mut audit, &error, 0, 0, 0);
                return Err(error);
            }
        };
        let preview = preview_batch_inner(&state, &request, None)?;
        if preview.eligible {
            audit.success(preview.member_ids.len() as u32, 0, preview.files);
        } else {
            audit.rejected(
                "batch_ineligible",
                0,
                preview.member_ids.len() as u32,
                preview.files,
            );
        }
        Ok(preview)
    })();
    finish_command_result(&mut audit, result)
}

/// Execute one batch through the same Guard coordinator and multi-file transaction engine.
fn execute_batch_inner(
    app: &AppHandle,
    state: &AppState,
    request: BatchRequest,
    preview_id: Option<String>,
    migration_parameter_id: Option<&str>,
    audit: &mut OperationAuditGuard<'_>,
) -> Result<BatchReport, String> {
    if !request.is_supported_version() {
        audit.rejected("guard_contract_version_unsupported", 0, 0, 0);
        return Err("guard_contract_version_unsupported".to_string());
    }
    let _write = match state.guard_coordinator.try_guard_write() {
        Ok(write) => write,
        Err(error) => {
            record_command_error(audit, &error, 0, 0, 0);
            return Err(error);
        }
    };
    let config = match state.config_store.load_launcher() {
        Ok(config) => config,
        Err(error) => {
            record_command_error(audit, &error, 0, 0, 0);
            return Err(error);
        }
    };
    if let Err(error) = reject_pending_format_migration(&config) {
        audit.rejected("format_migration_pending", 0, 0, 0);
        return Err(error);
    }
    if let Err(error) = validate_migration_override(&config, migration_parameter_id) {
        audit.rejected("lifecycle_migration_not_pending", 0, 0, 0);
        return Err(error);
    }
    let revision = match config_revision(&config) {
        Ok(revision) => revision,
        Err(error) => {
            record_command_error(audit, &error, 0, 0, 0);
            return Err(error);
        }
    };
    let progress_id = preview_id
        .clone()
        .unwrap_or_else(super::journal::new_batch_id);
    emit_batch_progress(app, &progress_id, GuardOperationPhase::Preflight, 1);
    if matches!(request.action, BatchAction::Apply | BatchAction::Lock) {
        let Some(preview_id) = preview_id.as_deref() else {
            audit.rejected("batch_preview_required", 0, 0, 0);
            return Err("batch_preview_required".to_string());
        };
        if !state
            .guard_coordinator
            .take_preview(preview_id, &request, &revision)
        {
            audit.rejected("batch_preview_stale", 0, 0, 0);
            return Err("batch_preview_stale".to_string());
        }
    }

    let contexts = match load_batch_context_from_config(
        &state.paths,
        &state.config_store,
        &config,
        migration_parameter_id,
    ) {
        Ok(contexts) => contexts,
        Err(error) => {
            record_command_error(audit, &error, 0, 0, 0);
            return Err(error);
        }
    };
    let roles = if scope_includes_roles(&request.scope) {
        match load_batch_roles(&state.paths, &state.config_store, &config, request.action) {
            Ok(roles) => roles,
            Err(error) => {
                record_command_error(audit, &error, 0, 0, 0);
                return Err(error);
            }
        }
    } else {
        Vec::new()
    };
    let selected = match select_batch_context(&request.scope, &contexts, &roles) {
        Ok(selected) => selected,
        Err(error) => {
            let result = if error == "batch_scope_empty" {
                OperationAuditResult::Rejected
            } else {
                OperationAuditResult::Failed
            };
            audit.record(result, Some(&stable_operation_error_code(&error)), 0, 0, 0);
            return Err(error);
        }
    };
    let selected_count = (selected.params.len() + selected.roles.len()) as u32;
    for context in &selected.params {
        if let Some(reason) = eligibility_reason(context, request.action) {
            audit.rejected(reason.as_str(), 0, selected_count, 0);
            return Err(reason.as_str().to_string());
        }
    }
    for role in &selected.roles {
        let eligibility = evaluate_eligibility(
            request.action,
            super::batch::EligibilityContext::new(
                role_lifecycle_as_parameter(role.lifecycle()),
                role_health_as_parameter(role.health),
            ),
        );
        if let Some(reason) = eligibility.reason {
            audit.rejected(reason.as_str(), 0, selected_count, 0);
            return Err(reason.as_str().to_string());
        }
    }

    let report_id = preview_id.unwrap_or_else(super::journal::new_batch_id);
    emit_batch_progress(app, &report_id, GuardOperationPhase::Snapshot, 2);
    let plans = if matches!(request.action, BatchAction::Apply | BatchAction::Lock) {
        match prepare_batch_plans(&state.paths, &selected, &roles, request.action) {
            Ok(plans) => plans,
            Err(error) => {
                record_command_error(audit, &error, 0, selected_count, 0);
                return Err(error);
            }
        }
    } else {
        Vec::new()
    };
    let plan_by_id = plans
        .iter()
        .flat_map(|(_, _, plan)| {
            plan.post_checks
                .iter()
                .map(move |check| (check.id.clone(), plan.changed))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let selected_ids = selected
        .params
        .iter()
        .map(|context| context.param.id.clone())
        .chain(selected.roles.iter().map(|role| role.id().to_string()))
        .collect::<Vec<_>>();
    let result = state.config_store.with_launcher_transaction(|launcher| {
        let now = now_secs();
        let guard = &mut launcher.config_mut().codex_guard;
        for context in &selected.params {
            let action_plan = plan_member_action(request.action, context.lifecycle, context.health);
            if action_plan.lifecycle == ParameterLifecycle::Disabled
                || migration_parameter_id == Some(context.param.id.as_str())
            {
                guard.pending_lifecycle_migrations.remove(&context.param.id);
            }
            let state = guard.params.entry(context.param.id.clone()).or_default();
            state.set_lifecycle(action_plan.lifecycle);
            state.last_checked = Some(now);
            if plan_by_id.get(&context.param.id).copied().unwrap_or(false) {
                state.last_restored = Some(now);
            }
        }
        for role in &selected.roles {
            let action_plan = plan_member_action(
                request.action,
                role_lifecycle_as_parameter(role.lifecycle()),
                role_health_as_parameter(role.health),
            );
            let lifecycle = match action_plan.lifecycle {
                ParameterLifecycle::Disabled => RoleLifecycle::Disabled,
                ParameterLifecycle::Applied => RoleLifecycle::Applied,
                ParameterLifecycle::Locked => RoleLifecycle::Locked,
            };
            let stored = guard
                .roles
                .iter_mut()
                .find(|state| state.id() == role.state.id())
                .ok_or_else(|| "role_not_managed".to_string())?;
            stored.lifecycle = lifecycle;
        }
        let codex_writer = PlatformAtomicFileWriter;
        emit_batch_progress(app, &report_id, GuardOperationPhase::Write, 3);
        let mut writes = plans
            .iter()
            .filter(|(_, _, plan)| plan.changed)
            .map(|(target, original, plan)| TransactionWrite {
                participant: JournalParticipant::Codex,
                relative_file: plan.relative_file.clone(),
                target: target.clone(),
                original: original.clone(),
                candidate_exists: true,
                candidate: plan.candidate.clone(),
                writer: &codex_writer,
            })
            .collect::<Vec<_>>();
        writes.push(TransactionWrite {
            participant: JournalParticipant::Launcher,
            relative_file: "config.json".to_string(),
            target: launcher.target().to_path_buf(),
            original: launcher.original().map(ToOwned::to_owned),
            candidate_exists: true,
            candidate: launcher.candidate_bytes()?,
            writer: launcher.writer(),
        });
        execute_transaction_batch(&state.paths, writes)
    });
    match result {
        Ok(()) => {
            emit_batch_progress(app, &report_id, GuardOperationPhase::Verify, 4);
            let changed = selected
                .params
                .iter()
                .filter(|context| {
                    plan_member_action(request.action, context.lifecycle, context.health).changed
                })
                .count()
                + selected
                    .roles
                    .iter()
                    .filter(|role| {
                        plan_member_action(
                            request.action,
                            role_lifecycle_as_parameter(role.lifecycle()),
                            role_health_as_parameter(role.health),
                        )
                        .changed
                    })
                    .count();
            let changed = changed as u32;
            let report = BatchReport {
                schema_version: super::batch::BATCH_CONTRACT_SCHEMA_VERSION,
                batch_id: report_id.clone(),
                outcome: BatchOutcome::Committed,
                changed,
                unchanged: selected_ids.len() as u32 - changed,
                files: plans.len() as u32,
                diagnostics: Vec::new(),
            };
            audit.success(report.changed, report.unchanged, report.files);
            emit_batch_progress(app, &report_id, GuardOperationPhase::Completed, 6);
            Ok(report)
        }
        Err(error) => {
            let is_critical = is_recovery_blocking_error(&error);
            if is_critical {
                state
                    .guard_coordinator
                    .mark_recovery_blocked("recovery_failed");
                audit.critical(
                    stable_batch_error_code(&error),
                    0,
                    selected_ids.len() as u32,
                    plans.len() as u32,
                );
                emit_batch_progress(app, &report_id, GuardOperationPhase::Recovery, 5);
            } else {
                audit.rolled_back(
                    stable_batch_error_code(&error),
                    0,
                    selected_ids.len() as u32,
                    plans.len() as u32,
                );
                emit_batch_progress(app, &report_id, GuardOperationPhase::Verify, 4);
            }
            let result = return_guard_transaction_result::<()>(state, Err(error.clone()));
            result.map(|_| unreachable!()).map_err(|_| error)
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn guard_execute_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    request: BatchRequest,
    preview_id: Option<String>,
) -> Result<BatchReport, String> {
    let mut audit = OperationAuditGuard::new(
        &state.paths,
        format!("batch:execute:{}", audit_scope(&request.scope)),
        match &request.scope {
            BatchScope::Role { role_id } => Some(role_id.as_str()),
            _ => None,
        },
    );
    let result = execute_batch_inner(&app, &state, request, preview_id, None, &mut audit);
    finish_command_result(&mut audit, result)
}

fn normalize_group_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    let length = trimmed.chars().count();
    if length == 0 || length > 80 || trimmed.contains('\0') {
        return Err("group_name_invalid".to_string());
    }
    Ok(trimmed.to_string())
}

fn custom_group_id(name: &str) -> String {
    let slug = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    format!("custom.{}", if slug.is_empty() { "group" } else { &slug })
}

fn duplicate_group_name(groups: &[GuardGroup], name: &str, except_id: Option<&str>) -> bool {
    groups
        .iter()
        .any(|group| Some(group.id.as_str()) != except_id && group.name.eq_ignore_ascii_case(name))
}

#[tauri::command]
#[specta::specta]
pub fn guard_group_create(state: State<'_, AppState>, name: String) -> Result<GuardGroup, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "group:create", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let name = normalize_group_name(&name)?;
        let result = return_guard_transaction_result(
            state.inner(),
            state.config_store.update_launcher(|config| {
                config.codex_guard.normalize_groups();
                if duplicate_group_name(&config.codex_guard.groups, &name, None) {
                    return Err("group_name_duplicate".to_string());
                }
                let mut id = custom_group_id(&name);
                let mut suffix = 2u32;
                while config.codex_guard.groups.iter().any(|group| group.id == id) {
                    id = format!("{}.{}", custom_group_id(&name), suffix);
                    suffix = suffix.saturating_add(1);
                }
                let order = config
                    .codex_guard
                    .groups
                    .iter()
                    .filter(|group| !group.builtin)
                    .map(|group| group.order)
                    .max()
                    .unwrap_or(1)
                    .saturating_add(1);
                let group = GuardGroup {
                    id,
                    name,
                    builtin: false,
                    order,
                };
                config.codex_guard.groups.push(group.clone());
                Ok(group)
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(result)
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_group_rename(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<GuardGroup, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "group:rename", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let name = normalize_group_name(&name)?;
        let result = return_guard_transaction_result(
            state.inner(),
            state.config_store.update_launcher(|config| {
                config.codex_guard.normalize_groups();
                let index = config
                    .codex_guard
                    .groups
                    .iter()
                    .position(|group| group.id == id)
                    .ok_or_else(|| "group_not_found".to_string())?;
                if config.codex_guard.groups[index].builtin {
                    return Err("builtin_group_immutable".to_string());
                }
                let duplicate = duplicate_group_name(&config.codex_guard.groups, &name, Some(&id));
                if duplicate {
                    return Err("group_name_duplicate".to_string());
                }
                config.codex_guard.groups[index].name = name;
                Ok(config.codex_guard.groups[index].clone())
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(result)
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_group_reorder(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<GuardGroup>, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "group:reorder", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let result = return_guard_transaction_result(
            state.inner(),
            state.config_store.update_launcher(|config| {
                config.codex_guard.normalize_groups();
                let custom = config
                    .codex_guard
                    .groups
                    .iter()
                    .filter(|group| !group.builtin)
                    .map(|group| group.id.clone())
                    .collect::<BTreeSet<_>>();
                let requested = ids.iter().cloned().collect::<BTreeSet<_>>();
                if requested != custom || requested.len() != ids.len() {
                    return Err("group_reorder_invalid".to_string());
                }
                for (offset, id) in ids.iter().enumerate() {
                    let group = config
                        .codex_guard
                        .groups
                        .iter_mut()
                        .find(|group| group.id == *id)
                        .ok_or_else(|| "group_not_found".to_string())?;
                    group.order = (offset as u32).saturating_add(2);
                }
                config.codex_guard.groups.sort_by_key(|group| {
                    (
                        if group.builtin { 0 } else { 1 },
                        group.order,
                        group.id.clone(),
                    )
                });
                Ok(config.codex_guard.groups.clone())
            }),
        )?;
        audit.success(result.len().min(u32::MAX as usize) as u32, 0, 0);
        Ok(result)
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_group_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "group:delete", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let schema = load_schema(&state.config_store)?;
        if schema
            .iter()
            .any(|param| param.group_id.as_deref() == Some(id.as_str()))
        {
            return Err("group_not_empty".to_string());
        }
        return_guard_transaction_result(
            state.inner(),
            state.config_store.update_launcher(|config| {
                config.codex_guard.normalize_groups();
                let position = config
                    .codex_guard
                    .groups
                    .iter()
                    .position(|group| group.id == id)
                    .ok_or_else(|| "group_not_found".to_string())?;
                if config.codex_guard.groups[position].builtin {
                    return Err("builtin_group_immutable".to_string());
                }
                config.codex_guard.groups.remove(position);
                Ok(())
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(())
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_parameter_move(
    state: State<'_, AppState>,
    parameter_id: String,
    group_id: String,
) -> Result<GuardParam, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "parameter:move", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let config = state.config_store.load_launcher()?;
        let target_group = config
            .codex_guard
            .groups
            .iter()
            .find(|group| group.id == group_id)
            .cloned()
            .ok_or_else(|| "group_not_found".to_string())?;
        if target_group.builtin {
            return Err("builtin_group_immutable".to_string());
        }
        let schema = load_schema(&state.config_store)?;
        let mut candidate = schema.clone();
        let param = candidate
            .iter_mut()
            .find(|param| param.id == parameter_id)
            .ok_or_else(|| "parameter_not_found".to_string())?;
        if !param.custom {
            return Err("builtin_parameter_immutable".to_string());
        }
        param.group_id = Some(group_id);
        let files = load_files(&state.config_store)?;
        validate_configuration(&state.paths, &files, &candidate)?;
        let moved = candidate
            .iter()
            .find(|param| param.id == parameter_id)
            .cloned()
            .ok_or_else(|| "parameter_not_found".to_string())?;
        return_guard_transaction_result(
            state.inner(),
            update_disk_schema(&state.config_store, |disk| {
                let slot = disk
                    .iter_mut()
                    .find(|param| param.id == parameter_id)
                    .ok_or_else(|| "parameter_not_found".to_string())?;
                slot.group_id = moved.group_id.clone();
                Ok(())
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(moved)
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_lifecycle_migration_resolve(
    app: AppHandle,
    state: State<'_, AppState>,
    parameter_id: String,
    choice: LifecycleMigrationChoice,
) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "lifecycle:migration:resolve", None);
    let result = (|| {
        let files = match choice {
            LifecycleMigrationChoice::Disabled => {
                let _write = state.guard_coordinator.try_guard_write()?;
                let config = state.config_store.load_launcher()?;
                if !config
                    .codex_guard
                    .pending_lifecycle_migrations
                    .contains_key(&parameter_id)
                {
                    return Err("lifecycle_migration_not_pending".to_string());
                }
                return_guard_transaction_result(
                    state.inner(),
                    state.config_store.update_launcher(|config| {
                        config
                            .codex_guard
                            .pending_lifecycle_migrations
                            .remove(&parameter_id);
                        config
                            .codex_guard
                            .params
                            .entry(parameter_id)
                            .or_default()
                            .set_lifecycle(ParameterLifecycle::Disabled);
                        Ok(())
                    }),
                )?;
                0
            }
            LifecycleMigrationChoice::Apply => {
                // Keep the pending marker until the shared batch transaction commits. The
                // internal override lets preview/execute plan this one member without making
                // the migration resolution a separate config write.
                execute_parameter_batch(
                    &app,
                    &state,
                    &parameter_id,
                    BatchAction::Apply,
                    Some(&parameter_id),
                    &mut audit,
                )?
                .files
            }
        };
        audit.success(1, 0, files);
        Ok(())
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_file_format_migration_resolve(
    state: State<'_, AppState>,
    file_id: String,
    format: GuardFileFormat,
) -> Result<GuardFile, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "file:migration:resolve", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let file = return_guard_transaction_result(
            state.inner(),
            state.config_store.update_launcher(|config| {
                let pending = config
                    .codex_guard
                    .pending_format_migrations
                    .get(&file_id)
                    .cloned()
                    .ok_or_else(|| "format_migration_not_pending".to_string())?;
                if !pending.candidates.contains(&format) {
                    return Err("format_migration_format_not_allowed".to_string());
                }
                let mut files = if config.codex_guard.files.is_empty() {
                    builtin_files()
                } else {
                    std::mem::take(&mut config.codex_guard.files)
                };
                let file = GuardFile {
                    id: pending.id.clone(),
                    name: pending.name,
                    file: pending.file,
                    format,
                    builtin: pending.builtin,
                    detection: pending.detection,
                };
                if let Some(existing) = files.iter_mut().find(|candidate| candidate.id == file_id) {
                    *existing = file.clone();
                } else {
                    files.push(file.clone());
                }
                config.codex_guard.files = files;
                config
                    .codex_guard
                    .pending_format_migrations
                    .remove(&file_id);
                Ok(file)
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(file)
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "guard:set_enabled", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        if enabled {
            let config = state.config_store.load_launcher()?;
            reject_pending_format_migration(&config)?;
            let files = load_files(&state.config_store)?;
            let schema = load_schema(&state.config_store)?;
            validate_configuration(&state.paths, &files, &schema)?;
        }
        return_guard_transaction_result(
            state.inner(),
            state.config_store.update_launcher(|config| {
                config.codex_guard.enabled = enabled;
                Ok(())
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(())
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_value(
    state: State<'_, AppState>,
    id: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "parameter:set_value", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let schema = load_schema(&state.config_store)?;
        let p = find_param(&schema, &id)?;
        let type_ok = match p.value_type.as_str() {
            "bool" => value.is_boolean(),
            "int" => value.as_i64().is_some(),
            "string" | "text" => value.is_string(),
            other => {
                return Err(trf(
                    "Parameter type {type} is not editable",
                    &[("type", other.to_string())],
                ))
            }
        };
        if !type_ok {
            return Err(tr("Value type mismatch"));
        }
        return_guard_transaction_result(
            state.inner(),
            state.config_store.update_launcher(|config| {
                let st = config.codex_guard.params.entry(id.clone()).or_default();
                if st.locked {
                    return Err(tr("Parameter is locked; unlock it before modifying"));
                }
                st.value = Some(value);
                Ok(())
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(())
    })();
    finish_command_result(&mut audit, result)
}

fn execute_parameter_batch(
    app: &AppHandle,
    state: &AppState,
    parameter_id: &str,
    action: BatchAction,
    migration_parameter_id: Option<&str>,
    audit: &mut OperationAuditGuard<'_>,
) -> Result<BatchReport, String> {
    let request = BatchRequest {
        schema_version: super::batch::BATCH_CONTRACT_SCHEMA_VERSION,
        scope: BatchScope::Parameter {
            parameter_id: parameter_id.to_string(),
        },
        action,
    };
    let preview_id = if action.writes_target_file() {
        let _write = state.guard_coordinator.try_guard_write()?;
        let preview = preview_batch_inner(state, &request, migration_parameter_id)?;
        if !preview.eligible {
            return Err("batch_preview_rejected".to_string());
        }
        Some(preview.preview_id)
    } else {
        None
    };
    execute_batch_inner(
        app,
        state,
        request,
        preview_id,
        migration_parameter_id,
        audit,
    )
}

#[tauri::command]
#[specta::specta]
pub fn guard_apply(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "parameter:apply", None);
    let result = execute_parameter_batch(&app, &state, &id, BatchAction::Apply, None, &mut audit)
        .map(|_| ());
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_applied(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    applied: bool,
) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "parameter:set_applied", None);
    let action = if applied {
        BatchAction::Apply
    } else {
        BatchAction::Disable
    };
    let result = execute_parameter_batch(&app, &state, &id, action, None, &mut audit).map(|_| ());
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_locked(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    locked: bool,
) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "parameter:set_locked", None);
    let action = if locked {
        BatchAction::Lock
    } else {
        BatchAction::Unlock
    };
    let result = execute_parameter_batch(&app, &state, &id, action, None, &mut audit).map(|_| ());
    finish_command_result(&mut audit, result)
}

// ============ 自定义参数管理 ============

#[tauri::command]
#[specta::specta]
pub fn guard_add_custom_param(
    state: State<'_, AppState>,
    mut param: GuardParam,
    file_id: String,
) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "parameter:add_custom", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let files = load_files(&state.config_store)?;
        let f = find_file(&files, &file_id)
            .ok_or_else(|| trf("Target file not found: {id}", &[("id", file_id.clone())]))?;

        param.id = normalize_custom_id(&param.id);
        param.custom = true;
        param.file = f.file.clone();
        param.file_id = f.id.clone();
        validate_param_fields(&param)?;
        validate_param_for_file(&param, f.format)?;

        let mut candidate_schema = load_schema(&state.config_store)?;
        if let Some(slot) = candidate_schema.iter_mut().find(|p| p.id == param.id) {
            *slot = param.clone();
        } else {
            candidate_schema.push(param.clone());
        }
        validate_configuration(&state.paths, &files, &candidate_schema)?;

        return_guard_transaction_result(
            state.inner(),
            update_disk_schema(&state.config_store, |disk| {
                if let Some(slot) = disk.iter_mut().find(|p| p.id == param.id) {
                    *slot = param;
                } else {
                    disk.push(param);
                }
                Ok(())
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(())
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_remove_custom_param(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "parameter:remove_custom", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let normalized = normalize_custom_id(&id);

        let files = load_files(&state.config_store)?;
        let mut candidate_schema = load_schema(&state.config_store)?;
        let before = candidate_schema.len();
        candidate_schema.retain(|param| param.id != normalized);
        if candidate_schema.len() == before {
            return Err(trf(
                "Custom parameter not found: {id}",
                &[("id", normalized.clone())],
            ));
        }
        validate_configuration(&state.paths, &files, &candidate_schema)?;

        return_guard_transaction_result(
            state.inner(),
            state
                .config_store
                .with_launcher_and_schema_transaction(|config, disk| {
                    let disk_before = disk.len();
                    disk.retain(|p| p.id != normalized);
                    if disk.len() == disk_before {
                        return Err(trf(
                            "Custom parameter not found: {id}",
                            &[("id", normalized.clone())],
                        ));
                    }
                    config.codex_guard.params.remove(&normalized);
                    Ok(())
                }),
        )?;

        audit.success(1, 0, 0);
        Ok(())
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_get_schema_file_path(state: State<'_, AppState>) -> Result<String, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "schema:path", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        return_guard_transaction_result(state.inner(), ensure_schema_file(&state.config_store))?;
        let path = schema_file_path(&state.paths).to_string_lossy().to_string();
        audit.success(1, 0, 0);
        Ok(path)
    })();
    finish_command_result(&mut audit, result)
}

// ============ 文件管理命令 ============

#[tauri::command]
#[specta::specta]
pub fn guard_get_files(state: State<'_, AppState>) -> Result<Vec<GuardFile>, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "file:list", None);
    let result = (|| {
        let files = load_files(&state.config_store)?;
        let schema = load_schema(&state.config_store)?;
        validate_configuration(&state.paths, &files, &schema)?;
        let count = files.len().min(u32::MAX as usize) as u32;
        audit.success(0, count, count);
        Ok(files)
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_add_file(
    state: State<'_, AppState>,
    name: String,
    file: String,
    format: GuardFileFormat,
) -> Result<GuardFile, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "file:add", None);
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        // 从 name 推导 id slug（简单处理：非字母数字替换为 -，去首尾 -，小写）
        let slug = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        if slug.is_empty() {
            return Err(tr("File name must contain at least one letter or digit"));
        }
        let id = normalize_custom_id(&slug);

        let trimmed_file =
            normalize_relative_path(file.trim()).map_err(|error| error.to_string())?;
        let gf = GuardFile {
            id: id.clone(),
            name: name.trim().to_string(),
            file: trimmed_file.clone(),
            format,
            builtin: false,
            detection: None,
        };
        validate_guard_file(&gf)?;
        let schema = load_schema(&state.config_store)?;
        let result = return_guard_transaction_result(
            state.inner(),
            update_files(&state.config_store, |files| {
                // id 与路径冲突检查（同路径会让参数在两个分组里重复显示）
                if files.iter().any(|file| file.id == id) {
                    return Err(trf(
                        "A file with the same name already exists: {name}",
                        &[("name", name.clone())],
                    ));
                }
                if files.iter().any(|file| file.file == trimmed_file) {
                    return Err(trf(
                        "Path already in guard list: {path}",
                        &[("path", trimmed_file.clone())],
                    ));
                }
                files.push(gf.clone());
                validate_configuration(&state.paths, files, &schema)?;
                Ok(gf)
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(result)
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_update_file(
    state: State<'_, AppState>,
    id: String,
    name: String,
    file: String,
) -> Result<GuardFile, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "file:update", Some(&id));
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let files = load_files(&state.config_store)?;
        let idx = files
            .iter()
            .position(|f| f.id == id)
            .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;

        let old_file = files[idx].file.clone();
        let new_file = normalize_relative_path(file.trim()).map_err(|error| error.to_string())?;

        if files[idx].builtin && old_file != new_file {
            return Err(tr("Built-in file paths cannot be changed"));
        }

        if old_file != new_file && files.iter().any(|f| f.id != id && f.file == new_file) {
            return Err(trf(
                "Path already in guard list: {path}",
                &[("path", new_file.clone())],
            ));
        }

        let mut updated = files[idx].clone();
        updated.name = name.trim().to_string();
        updated.file = new_file.clone();
        if old_file != new_file {
            updated.detection = None;
        }
        validate_guard_file(&updated)?;

        let mut candidate_files = files.clone();
        candidate_files[idx] = updated.clone();
        let mut candidate_schema = load_schema(&state.config_store)?;
        if old_file != new_file {
            for param in &mut candidate_schema {
                if param.custom && param.file == old_file {
                    param.file = new_file.clone();
                }
            }
        }
        validate_configuration(&state.paths, &candidate_files, &candidate_schema)?;

        // 如果是自定义参数的归属文件，路径变了参数的 file 也要跟着变
        // schema 中该文件路径下的自定义参数需要更新 file 字段
        if old_file != new_file {
            let result = return_guard_transaction_result(
                state.inner(),
                state
                    .config_store
                    .with_launcher_and_schema_transaction(|config, disk| {
                        for param in disk {
                            if param.custom && param.file == old_file {
                                param.file = new_file.clone();
                            }
                        }
                        let mut current = if config.codex_guard.files.is_empty() {
                            builtin_files()
                        } else {
                            std::mem::take(&mut config.codex_guard.files)
                        };
                        if current
                            .iter()
                            .any(|file| file.id != id && file.file == new_file)
                        {
                            return Err(trf(
                                "Path already in guard list: {path}",
                                &[("path", new_file.clone())],
                            ));
                        }
                        let slot = current
                            .iter_mut()
                            .find(|file| file.id == id)
                            .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
                        *slot = updated.clone();
                        config.codex_guard.files = current;
                        Ok(updated.clone())
                    }),
            )?;
            audit.success(1, 0, 0);
            return Ok(result);
        }
        let result = return_guard_transaction_result(
            state.inner(),
            state
                .config_store
                .with_launcher_and_schema_transaction(|config, disk| {
                    let _ = disk;
                    let mut current = if config.codex_guard.files.is_empty() {
                        builtin_files()
                    } else {
                        std::mem::take(&mut config.codex_guard.files)
                    };
                    if current
                        .iter()
                        .any(|file| file.id != id && file.file == new_file)
                    {
                        return Err(trf(
                            "Path already in guard list: {path}",
                            &[("path", new_file.clone())],
                        ));
                    }
                    let slot = current
                        .iter_mut()
                        .find(|file| file.id == id)
                        .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
                    *slot = updated.clone();
                    config.codex_guard.files = current;
                    Ok(updated)
                }),
        )?;
        audit.success(1, 0, 0);
        Ok(result)
    })();
    finish_command_result(&mut audit, result)
}

#[tauri::command]
#[specta::specta]
pub fn guard_remove_file(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "file:remove", Some(&id));
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let files = load_files(&state.config_store)?;
        let idx = files
            .iter()
            .position(|file| file.id == id)
            .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
        if files[idx].builtin {
            return Err(tr("Built-in files cannot be removed"));
        }
        let target_file = files[idx].file.clone();
        let candidate_files = files
            .iter()
            .filter(|file| file.id != id)
            .cloned()
            .collect::<Vec<_>>();
        let schema = load_schema(&state.config_store)?;
        let candidate_schema = schema
            .into_iter()
            .filter(|param| !(param.custom && param.file == target_file))
            .collect::<Vec<_>>();
        validate_configuration(&state.paths, &candidate_files, &candidate_schema)?;

        return_guard_transaction_result(
            state.inner(),
            state
                .config_store
                .with_launcher_and_schema_transaction(|config, disk| {
                    let mut current = if config.codex_guard.files.is_empty() {
                        builtin_files()
                    } else {
                        std::mem::take(&mut config.codex_guard.files)
                    };
                    let current_index = current
                        .iter()
                        .position(|file| file.id == id)
                        .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
                    if current[current_index].builtin {
                        return Err(tr("Built-in files cannot be removed"));
                    }
                    current.remove(current_index);
                    config.codex_guard.files = current;

                    let removed_ids = disk
                        .iter()
                        .filter(|param| param.custom && param.file == target_file)
                        .map(|param| param.id.clone())
                        .collect::<Vec<_>>();
                    disk.retain(|param| !(param.custom && param.file == target_file));
                    for removed_id in removed_ids {
                        config.codex_guard.params.remove(&removed_id);
                    }
                    Ok(())
                }),
        )?;
        audit.success(1, 0, 0);
        Ok(())
    })();
    finish_command_result(&mut audit, result)
}

// ============ 路径检测 ============

/// 检测文件实际路径并落盘记录；之后直接读记录，不重复扫盘
#[tauri::command]
#[specta::specta]
pub fn guard_detect_file(state: State<'_, AppState>, id: String) -> Result<GuardFile, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "file:detect", Some(&id));
    let result = (|| {
        let _write = state.guard_coordinator.try_guard_write()?;
        let files = load_files(&state.config_store)?;
        let schema = load_schema(&state.config_store)?;
        validate_configuration(&state.paths, &files, &schema)?;
        let result = return_guard_transaction_result(
            state.inner(),
            update_files(&state.config_store, |files| {
                let file = files
                    .iter_mut()
                    .find(|file| file.id == id)
                    .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
                file.detection = Some(DetectRecord {
                    path: detect_file_path(&state.paths, &file.file),
                    at: now_secs(),
                });
                Ok(file.clone())
            }),
        )?;
        audit.success(1, 0, 0);
        Ok(result)
    })();
    finish_command_result(&mut audit, result)
}

/// 把文件选择器选中的绝对路径换算为相对 ~/.codex 的路径（越界拒绝）
#[tauri::command]
#[specta::specta]
pub fn guard_relativize_picked_path(
    state: State<'_, AppState>,
    abs_path: String,
) -> Result<String, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "file:relativize", None);
    let result = (|| {
        let home = state.paths.codex_root();
        let rel = std::path::Path::new(&abs_path)
            .strip_prefix(home)
            .map_err(|_| tr("Selected file must be inside ~/.codex"))?;
        let rel =
            normalize_relative_path(&rel.to_string_lossy()).map_err(|error| error.to_string())?;
        let result = validate_target_path(&state.paths, &rel).map_err(|error| error.to_string())?;
        audit.success(0, 1, 0);
        Ok(result)
    })();
    finish_command_result(&mut audit, result)
}

#[cfg(test)]
mod tests {
    use super::super::PendingLifecycleMigration;
    use super::*;
    use crate::codex_guard::roles::{
        build_managed_role_record, CapabilitySnapshot, ModelCapability, RoleEditableFields, RoleId,
        TextOrigin,
    };

    fn role_context() -> BatchRoleContext {
        let capabilities = CapabilitySnapshot::new(
            vec![ModelCapability::new("model-v2", "v2", vec!["low".to_string()]).unwrap()],
            "fixture",
            1,
        )
        .unwrap();
        let fields = RoleEditableFields {
            display_name: "Reviewer".to_string(),
            purpose: "Review changes".to_string(),
            selection_criteria: "review code".to_string(),
            model: "model-v2".to_string(),
            effort: "low".to_string(),
            instructions: "Review the change.".to_string(),
        };
        let record = build_managed_role_record(
            RoleId::new("reviewer").unwrap(),
            &fields,
            1,
            1,
            &capabilities,
            TextOrigin::User,
            TextOrigin::User,
        )
        .unwrap();
        BatchRoleContext {
            state: ManagedRoleState::new(record),
            health: RoleHealth::Healthy,
            actual_file_present: false,
        }
    }

    #[test]
    fn role_scope_selects_the_requested_managed_role() {
        let role = role_context();
        let selected = select_batch_context(
            &BatchScope::role("reviewer"),
            &[],
            std::slice::from_ref(&role),
        )
        .unwrap();
        assert!(selected.params.is_empty());
        assert_eq!(selected.roles.len(), 1);
        assert_eq!(selected.roles[0].id(), "reviewer");
    }

    #[test]
    fn role_and_shared_agents_member_are_planned_once_per_physical_file() {
        let temp = tempfile::tempdir().unwrap();
        let paths = super::super::AppPaths::for_test(temp.path());
        let role = role_context();
        let param = BatchParamContext {
            param: GuardParam {
                id: "agents-md-subagent-section".to_string(),
                label: "Subagent section".to_string(),
                label_en: String::new(),
                description: String::new(),
                description_en: String::new(),
                file: ROLE_DIRECTORY_FILE.to_string(),
                file_id: "builtin.agents-md".to_string(),
                group_id: Some(ROLE_GROUP_ID.to_string()),
                apply_mode: "markdown_block".to_string(),
                path: String::new(),
                value_type: "text".to_string(),
                default: serde_json::Value::String("Shared policy".to_string()),
                default_en: serde_json::Value::Null,
                custom: false,
            },
            format: GuardFileFormat::Markdown,
            expected: serde_json::Value::String("Shared policy".to_string()),
            lifecycle: ParameterLifecycle::Disabled,
            health: HealthStatus::Healthy,
        };
        let selected = SelectedBatch {
            params: vec![&param],
            roles: vec![&role],
        };
        let plans = prepare_batch_plans(
            &paths,
            &selected,
            std::slice::from_ref(&role),
            BatchAction::Apply,
        )
        .unwrap();
        assert_eq!(
            plans
                .iter()
                .filter(|(_, _, plan)| plan.relative_file == ROLE_DIRECTORY_FILE)
                .count(),
            1
        );
        assert_eq!(plans.len(), 2);
        let agents = plans
            .iter()
            .find(|(_, _, plan)| plan.relative_file == ROLE_DIRECTORY_FILE)
            .map(|(_, _, plan)| String::from_utf8_lossy(&plan.candidate).to_string())
            .unwrap();
        assert!(agents.contains("agents-md-subagent-section"));
        assert!(agents.contains(ROLE_DIRECTORY_MEMBER_ID));
        assert!(plans
            .iter()
            .any(|(_, _, plan)| plan.relative_file == "agents/reviewer.toml"));
    }

    #[test]
    fn stable_input_errors_are_audited_as_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let paths = super::super::AppPaths::for_test(temp.path());
        let cases = [
            ("group_name_duplicate", "group-duplicate"),
            ("group_not_empty", "group-delete"),
            (
                "lifecycle_migration_not_pending",
                "lifecycle-migration-resolve",
            ),
        ];

        for (error, scope) in cases {
            let mut audit = OperationAuditGuard::new(&paths, scope, None);
            record_command_error(&mut audit, error, 0, 0, 0);
        }

        let records = super::super::operation_audit::read_operation_audit(&paths).unwrap();
        assert_eq!(records.len(), cases.len());
        for (record, (error, scope)) in records.iter().zip(cases) {
            assert_eq!(record.scope, scope);
            assert_eq!(record.result, OperationAuditResult::Rejected);
            assert_eq!(record.error_code.as_deref(), Some(error));
        }
    }

    #[test]
    fn config_revision_ignores_runtime_timestamps_but_tracks_semantic_changes() {
        let mut config = LauncherConfig::default();
        let state = config
            .codex_guard
            .params
            .entry("features.demo".to_string())
            .or_default();
        state.applied = true;
        state.locked = true;
        state.value = Some(serde_json::Value::Bool(true));

        let baseline = config_revision(&config).unwrap();
        config
            .codex_guard
            .params
            .get_mut("features.demo")
            .unwrap()
            .last_checked = Some(123);
        config
            .codex_guard
            .params
            .get_mut("features.demo")
            .unwrap()
            .last_restored = Some(456);
        assert_eq!(config_revision(&config).unwrap(), baseline);

        config
            .codex_guard
            .params
            .get_mut("features.demo")
            .unwrap()
            .locked = false;
        assert_ne!(config_revision(&config).unwrap(), baseline);
    }

    #[test]
    fn migration_override_requires_the_pending_marker() {
        let mut config = LauncherConfig::default();
        assert_eq!(
            validate_migration_override(&config, Some("legacy")),
            Err("lifecycle_migration_not_pending".to_string())
        );

        config.codex_guard.pending_lifecycle_migrations.insert(
            "legacy".to_string(),
            PendingLifecycleMigration {
                applied: false,
                locked: true,
            },
        );
        assert!(validate_migration_override(&config, Some("legacy")).is_ok());
        assert!(validate_migration_override(&config, None).is_ok());
    }

    #[test]
    fn config_revision_is_stable_for_equivalent_hash_map_order() {
        let mut first = LauncherConfig::default();
        first.codex_guard.params.insert(
            "features.first".to_string(),
            GuardParamState {
                applied: true,
                ..GuardParamState::default()
            },
        );
        first.codex_guard.params.insert(
            "features.second".to_string(),
            GuardParamState {
                locked: true,
                ..GuardParamState::default()
            },
        );

        let mut second = LauncherConfig::default();
        second.codex_guard.params.insert(
            "features.second".to_string(),
            GuardParamState {
                locked: true,
                ..GuardParamState::default()
            },
        );
        second.codex_guard.params.insert(
            "features.first".to_string(),
            GuardParamState {
                applied: true,
                ..GuardParamState::default()
            },
        );

        assert_eq!(
            config_revision(&first).unwrap(),
            config_revision(&second).unwrap()
        );
    }
}
