//! Tauri role commands.
//!
//! These commands keep the role boundary narrow: inputs contain structured
//! fields, discovery returns summaries only, and all role-file paths are
//! derived from a validated RoleId. Model capability refresh is intentionally
//! supplied by the capability boundary; these commands never invent a
//! successful snapshot when the directory is unavailable.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::atomic_store::PlatformAtomicFileWriter;
use super::capability::{capability_with_ttl, probe_capability_fresh};
use crate::AppState;

use super::engine::{execute_transaction_batch, TransactionWrite};
use super::journal::JournalParticipant;
use super::operation_audit::OperationAuditGuard;
use super::transaction::is_recovery_blocking_error;

use super::roles::{
    parse_role_toml, policy_hash_for_view, render_minimal_role_toml, validate_copy_roles,
    validate_default_action, validate_role_id, CapabilitySnapshot, ManagedRoleRecord, RoleAction,
    RoleEditableFields, RoleError, RoleErrorCode, RoleId, RoleLifecycle, RoleMigrationChoice,
    RoleMigrationPlan, RoleMigrationResult, TextOrigin,
};
use super::roles_store::{
    content_sha256, discover_role_files, load_role_states, replace_role_directory_block,
    role_file_present, role_summaries, role_target, validate_role_states, ManagedRoleState,
};
use super::{now_secs, AppPaths};

fn stable_role_error_code(error: &str) -> String {
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

fn record_role_error(audit: &mut OperationAuditGuard<'_>, error: &str) {
    let code = stable_role_error_code(error);
    if error == "guard_busy" {
        audit.busy(code, 0, 0, 0);
    } else if is_recovery_blocking_error(error) {
        audit.critical(code, 0, 0, 0);
    } else if error.starts_with("role.")
        || error.starts_with("role_")
        || error.starts_with("capability_")
        || error == "role_not_managed"
    {
        audit.rejected(code, 0, 0, 0);
    } else if error.starts_with("guard transaction failed: ") {
        audit.rolled_back(code, 0, 0, 0);
    } else {
        audit.failure(code, 0, 0, 0);
    }
}

fn finish_role_command_result<T>(
    audit: &mut OperationAuditGuard<'_>,
    result: Result<T, String>,
) -> Result<T, String> {
    if let Err(error) = &result {
        record_role_error(audit, error);
    }
    result
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleSaveInput {
    pub id: String,
    pub display_name: String,
    pub purpose: String,
    pub selection_criteria: String,
    pub model: String,
    pub effort: String,
    pub instructions: String,
}

impl RoleSaveInput {
    fn fields(&self) -> RoleEditableFields {
        RoleEditableFields {
            display_name: self.display_name.clone(),
            purpose: self.purpose.clone(),
            selection_criteria: self.selection_criteria.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            instructions: self.instructions.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleCopyInput {
    pub id: String,
    pub display_name: String,
    pub selection_criteria: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleReorderInput {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleSummaryDto {
    pub id: String,
    pub display_name: String,
    pub purpose: String,
    pub selection_criteria: String,
    pub model: String,
    pub effort: String,
    pub order: u32,
    pub lifecycle: String,
    pub health: String,
    pub actual_file_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleDetailDto {
    pub id: String,
    pub display_name: String,
    pub purpose: String,
    pub selection_criteria: String,
    pub model: String,
    pub effort: String,
    pub instructions: String,
    pub lifecycle: String,
    pub health: String,
    pub actual_file_present: bool,
    pub policy_revision: u64,
    pub policy_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleListDto {
    pub roles: Vec<RoleSummaryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleDiscoveryDiagnosticDto {
    pub code: String,
    pub field: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleDiscoveryItemDto {
    pub id: String,
    pub relative_file_name: String,
    pub syntax: String,
    pub capability: String,
    pub can_adopt: bool,
    pub managed: bool,
    pub diagnostics: Vec<RoleDiscoveryDiagnosticDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleDiscoveryDto {
    pub roles: Vec<RoleDiscoveryItemDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleDeleteReport {
    pub id: String,
    pub file_removed: bool,
    pub directory_updated: bool,
    pub files_removed: u32,
    pub backup_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleMigrationResolveInput {
    pub choice: String,
    pub safe_default_toml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleMigrationDto {
    pub status: String,
    pub reason: Option<String>,
    pub choices: Vec<String>,
    pub role: Option<RoleSummaryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityModelDto {
    pub slug: String,
    pub multi_agent_version: String,
    pub efforts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySnapshotDto {
    pub available: bool,
    pub executable_identity: Option<String>,
    pub fetched_at_ms: Option<u64>,
    pub models: Vec<CapabilityModelDto>,
    /// Stable failure code when the most recent probe was unavailable but a
    /// prior successful snapshot is being shown offline.
    pub error: Option<String>,
}

fn parse_id(raw: &str) -> Result<RoleId, String> {
    validate_role_id(raw).map_err(|error| error.to_string())
}

fn role_fields_to_record(
    id: RoleId,
    fields: &RoleEditableFields,
    order: u32,
    revision: u64,
    previous_toml: Option<&str>,
) -> Result<ManagedRoleRecord, String> {
    let expected_toml = match previous_toml {
        Some(previous) => super::roles::update_role_toml(previous, fields),
        None => render_minimal_role_toml(fields),
    }
    .map_err(|error| error.to_string())?;
    let parsed = parse_role_toml(&expected_toml).map_err(|error| error.to_string())?;
    Ok(ManagedRoleRecord {
        id,
        selection_criteria: fields.selection_criteria.trim().to_string(),
        order,
        expected_toml,
        policy_revision: revision,
        policy_hash: policy_hash_for_view(&parsed.view, &fields.selection_criteria),
        policy_updated_at_ms: now_secs().saturating_mul(1000),
        description_origin: TextOrigin::User,
        instructions_origin: TextOrigin::User,
    })
}

fn stable_enum<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn summary_dto(summary: super::roles::ManagedRoleSummary) -> RoleSummaryDto {
    RoleSummaryDto {
        id: summary.id.to_string(),
        display_name: summary.display_name,
        purpose: summary.purpose,
        selection_criteria: summary.selection_criteria,
        model: summary.model,
        effort: summary.effort,
        order: summary.order,
        lifecycle: stable_enum(&summary.lifecycle),
        health: stable_enum(&summary.health),
        actual_file_present: summary.actual_file_present,
    }
}

fn state_detail(
    paths: &AppPaths,
    state: ManagedRoleState,
    capabilities: Option<&super::roles::CapabilitySnapshot>,
) -> Result<RoleDetailDto, String> {
    let present = role_file_present(paths, state.id())?;
    let managed = state
        .into_managed(present, capabilities)
        .map_err(|error| error.to_string())?;
    Ok(RoleDetailDto {
        id: managed.record.id.to_string(),
        display_name: managed.view.display_name,
        purpose: managed.view.purpose,
        selection_criteria: managed.record.selection_criteria,
        model: managed.view.model,
        effort: managed.view.effort,
        instructions: managed.view.instructions,
        lifecycle: stable_enum(&managed.lifecycle),
        health: stable_enum(&managed.health),
        actual_file_present: managed.actual_file_present,
        policy_revision: managed.record.policy_revision,
        policy_hash: managed.record.policy_hash,
    })
}

/// Write paths validate against a live probe; the fresh result also reseeds the shared
/// cache so a save right after a page load does not pay for a second probe.
fn refresh_capabilities(state: &AppState) -> Result<CapabilitySnapshot, String> {
    let config = state.config_store.load_launcher()?;
    probe_capability_fresh(&config.codex_app_path, now_secs().saturating_mul(1000)).result
}

/// Read paths share one probe through the TTL cache: role list, discovery, and the
/// capability view all render from the same page load.
fn optional_capabilities(state: &AppState) -> Option<CapabilitySnapshot> {
    let config = state.config_store.load_launcher().ok()?;
    capability_with_ttl(&config.codex_app_path, now_secs().saturating_mul(1000))
        .result
        .ok()
}

fn persist_capability_snapshot(state: &AppState, snapshot: CapabilitySnapshot) {
    // Persisting is a best-effort cache write: a real operation holding the write lock must
    // never fail a read-only capability query, and the next probe persists instead.
    let Ok(_write) = state.guard_coordinator.try_guard_write() else {
        return;
    };
    let result = state.config_store.update_launcher(|config| {
        config.codex_guard.capability_snapshot = Some(snapshot);
        Ok(())
    });
    if let Err(error) = &result {
        if is_recovery_blocking_error(error) {
            state
                .guard_coordinator
                .mark_recovery_blocked("recovery_failed");
        }
        log::error!("guard capability snapshot persist failed: {}", error);
    }
}

fn capability_command_result(
    state: &AppState,
    force_probe: bool,
) -> Result<CapabilitySnapshotDto, String> {
    let config = state.config_store.load_launcher()?;
    let now_ms = now_secs().saturating_mul(1000);
    let probe = if force_probe {
        probe_capability_fresh(&config.codex_app_path, now_ms)
    } else {
        capability_with_ttl(&config.codex_app_path, now_ms)
    };
    match probe.result {
        Ok(snapshot) => {
            // Persist only when the stored snapshot is stale: cache hits must not pay for
            // the write lock + fsync on every page load.
            if config.codex_guard.capability_snapshot.as_ref() != Some(&snapshot) {
                persist_capability_snapshot(state, snapshot.clone());
            }
            Ok(capability_snapshot_dto(snapshot, true, None))
        }
        Err(error) => match config.codex_guard.capability_snapshot {
            Some(snapshot) => Ok(capability_snapshot_dto(snapshot, false, Some(error))),
            None => Err(error),
        },
    }
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_get(state: State<'_, AppState>, id: String) -> Result<RoleDetailDto, String> {
    // Read-only query: never appends to the operation audit (see guard_operation_audit_list).
    let id = parse_id(&id)?;
    let states = load_role_states(&state.config_store)?;
    let role = states
        .into_iter()
        .find(|candidate| candidate.id() == &id)
        .ok_or_else(|| "role_not_managed".to_string())?;
    let capabilities = optional_capabilities(&state);
    state_detail(&state.paths, role, capabilities.as_ref())
}

fn role_list_impl(state: &AppState) -> Result<RoleListDto, String> {
    let states = load_role_states(&state.config_store)?;
    let capabilities = optional_capabilities(state);
    let summaries = role_summaries(&state.paths, &states, capabilities.as_ref())?
        .into_iter()
        .map(summary_dto)
        .collect();
    Ok(RoleListDto { roles: summaries })
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_list(state: State<'_, AppState>) -> Result<RoleListDto, String> {
    role_list_impl(&state)
}

fn role_discover_impl(state: &AppState) -> Result<RoleDiscoveryDto, String> {
    let states = load_role_states(&state.config_store)?;
    let capabilities = optional_capabilities(state);
    let discovered = discover_role_files(&state.paths, &states, capabilities.as_ref())?;
    Ok(RoleDiscoveryDto {
        roles: discovered.into_iter().map(discovery_item_dto).collect(),
    })
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_discover(state: State<'_, AppState>) -> Result<RoleDiscoveryDto, String> {
    role_discover_impl(&state)
}

fn discovery_item_dto(item: super::roles::DiscoveredRole) -> RoleDiscoveryItemDto {
    RoleDiscoveryItemDto {
        id: item.id,
        relative_file_name: item.relative_file_name,
        syntax: stable_enum(&item.syntax),
        capability: stable_enum(&item.capability),
        can_adopt: item.can_adopt,
        managed: item.managed,
        diagnostics: item
            .diagnostics
            .into_iter()
            .map(|diagnostic| RoleDiscoveryDiagnosticDto {
                code: stable_enum(&diagnostic.code),
                field: diagnostic.field,
            })
            .collect(),
    }
}

/// Commit role state, the generated role directory and any role-file changes as
/// one durable Guard transaction. Role commands must not write launcher state
/// separately from the Codex files they describe.
fn commit_role_states(state: &AppState, next_states: Vec<ManagedRoleState>) -> Result<(), String> {
    validate_role_states(&next_states).map_err(|error| error.to_string())?;

    let mut present_ids = HashSet::new();
    for managed in &next_states {
        if role_file_present(&state.paths, managed.id())? {
            present_ids.insert(managed.id().to_string());
        }
    }
    let agents_before_bytes = read_agents_snapshot(&state.paths)?;
    let agents_before = document_from_snapshot(agents_before_bytes.as_deref())?;
    let agents_after = replace_role_directory_block(&agents_before, &next_states, &present_ids)?;

    let result = state.config_store.with_launcher_transaction(|launcher| {
        launcher.config_mut().codex_guard.roles = next_states.clone();
        let codex_writer = PlatformAtomicFileWriter;
        let writes = vec![
            TransactionWrite {
                participant: JournalParticipant::Codex,
                relative_file: "AGENTS.md".to_string(),
                target: state.paths.codex_file("AGENTS.md"),
                original: agents_before_bytes.clone(),
                candidate_exists: true,
                candidate: agents_after.as_bytes().to_vec(),
                writer: &codex_writer,
            },
            TransactionWrite {
                participant: JournalParticipant::Launcher,
                relative_file: "config.json".to_string(),
                target: launcher.target().to_path_buf(),
                original: launcher.original().map(ToOwned::to_owned),
                candidate_exists: true,
                candidate: launcher.candidate_bytes()?,
                writer: launcher.writer(),
            },
        ];
        execute_transaction_batch(&state.paths, writes)
    });
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

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_save(
    state: State<'_, AppState>,
    input: RoleSaveInput,
) -> Result<RoleDetailDto, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "role:save", Some(&input.id));
    let result = (|| {
        let id = parse_id(&input.id)?;
        let fields = input.fields();
        let _write = match state.guard_coordinator.try_guard_write() {
            Ok(write) => write,
            Err(error) => {
                record_role_error(&mut audit, &error);
                return Err(error);
            }
        };
        let capabilities = refresh_capabilities(&state)?;
        super::roles::validate_editable_fields(&fields, &capabilities)
            .map_err(|error| error.to_string())?;

        let states = load_role_states(&state.config_store)?;
        let existing = states.iter().find(|candidate| candidate.id() == &id);
        if existing.is_none() && role_file_present(&state.paths, &id)? {
            // A live file with this id belongs to discovery/adoption.  Creating a
            // new managed expectation must never silently claim or overwrite it.
            return Err(RoleError::new(RoleErrorCode::DuplicateRoleId).to_string());
        }
        let order = existing
            .map(|candidate| candidate.record.order)
            .unwrap_or(states.len() as u32);
        let revision = existing
            .map(|candidate| candidate.record.policy_revision.saturating_add(1))
            .unwrap_or(1);
        let previous_toml = existing.map(|candidate| candidate.record.expected_toml.as_str());
        let record = role_fields_to_record(id.clone(), &fields, order, revision, previous_toml)?;
        let lifecycle = existing
            .map(|candidate| candidate.lifecycle)
            .unwrap_or(RoleLifecycle::Disabled);

        let mut next_states = states;
        if let Some(slot) = next_states
            .iter_mut()
            .find(|candidate| candidate.id() == &id)
        {
            slot.record = record.clone();
        } else {
            next_states.push(ManagedRoleState {
                record: record.clone(),
                lifecycle,
            });
        }
        // Save only changes the Guard expectation.  The role file is materialized
        // by the first Apply/Lock batch; an existing file is intentionally left
        // untouched and will surface as drift until that action is run.
        if let Err(error) = commit_role_states(&state, next_states) {
            record_role_error(&mut audit, &error);
            return Err(error);
        }
        let updated = ManagedRoleState { record, lifecycle };
        let detail = state_detail(&state.paths, updated, Some(&capabilities))?;
        audit.success(1, 0, 1);
        Ok(detail)
    })();
    finish_role_command_result(&mut audit, result)
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_copy(
    state: State<'_, AppState>,
    source: String,
    input: RoleCopyInput,
) -> Result<RoleDetailDto, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "role:copy", Some(&input.id));
    let result = (|| {
        let source_id = parse_id(&source)?;
        let target_id = parse_id(&input.id)?;
        let _write = match state.guard_coordinator.try_guard_write() {
            Ok(write) => write,
            Err(error) => {
                record_role_error(&mut audit, &error);
                return Err(error);
            }
        };
        let capabilities = refresh_capabilities(&state)?;
        validate_copy_roles(&source_id, &target_id).map_err(|error| error.to_string())?;

        let states = load_role_states(&state.config_store)?;
        if states.iter().any(|candidate| candidate.id() == &target_id) {
            return Err(RoleError::new(RoleErrorCode::DuplicateRoleId).to_string());
        }
        if role_file_present(&state.paths, &target_id)? {
            return Err(RoleError::new(RoleErrorCode::DuplicateRoleId).to_string());
        }
        let source_state = states
            .iter()
            .find(|candidate| candidate.id() == &source_id)
            .ok_or_else(|| "role_not_managed".to_string())?;
        let view = source_state
            .record
            .view()
            .map_err(|error| error.to_string())?;
        let fields = RoleEditableFields {
            display_name: input.display_name,
            purpose: view.purpose,
            selection_criteria: input.selection_criteria,
            model: view.model,
            effort: view.effort,
            instructions: view.instructions,
        };
        let record =
            role_fields_to_record(target_id.clone(), &fields, states.len() as u32, 1, None)?;
        super::roles::validate_editable_fields(&fields, &capabilities)
            .map_err(|error| error.to_string())?;
        let mut next_states = states;
        next_states.push(ManagedRoleState::new(record.clone()));
        // Copy persists only the new Guard expectation.  Do not create the
        // target role file until Apply/Lock is explicitly requested.
        if let Err(error) = commit_role_states(&state, next_states) {
            record_role_error(&mut audit, &error);
            return Err(error);
        }
        let detail = state_detail(
            &state.paths,
            ManagedRoleState::new(record),
            Some(&capabilities),
        )?;
        audit.success(1, 0, 1);
        Ok(detail)
    })();
    finish_role_command_result(&mut audit, result)
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_adopt(state: State<'_, AppState>, id: String) -> Result<RoleDetailDto, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "role:adopt", Some(&id));
    let result = (|| {
        let id = parse_id(&id)?;
        let _write = match state.guard_coordinator.try_guard_write() {
            Ok(write) => write,
            Err(error) => {
                record_role_error(&mut audit, &error);
                return Err(error);
            }
        };
        let capabilities = refresh_capabilities(&state)?;
        let states = load_role_states(&state.config_store)?;
        if states.iter().any(|candidate| candidate.id() == &id) {
            return Err(RoleError::new(RoleErrorCode::DuplicateRoleId).to_string());
        }
        if states.len() >= super::roles::MAX_MANAGED_ROLES {
            return Err(RoleError::new(RoleErrorCode::TooManyManagedRoles).to_string());
        }
        let first = role_file_present(&state.paths, &id)?;
        if !first {
            return Err(RoleError::new(RoleErrorCode::MigrationSourceUnavailable).to_string());
        }
        let bytes = super::roles_store::read_role_file(&state.paths, &id)?
            .ok_or_else(|| "role_file_read_failed".to_string())?;
        let first_hash = content_sha256(&bytes);
        let parsed = parse_role_toml(
            std::str::from_utf8(&bytes)
                .map_err(|_| RoleError::new(RoleErrorCode::InvalidText).to_string())?,
        )
        .map_err(|error| error.to_string())?;
        super::roles::validate_role_capability(&parsed.view, &capabilities)
            .map_err(|error| error.to_string())?;
        let second = super::roles_store::read_role_file(&state.paths, &id)?
            .ok_or_else(|| "role_file_read_failed".to_string())?;
        if content_sha256(&second) != first_hash {
            return Err("role_file_changed".to_string());
        }
        let now = now_secs().saturating_mul(1000);
        let record = ManagedRoleRecord {
            id: id.clone(),
            selection_criteria: parsed.view.purpose.clone(),
            order: states.len() as u32,
            expected_toml: String::from_utf8(bytes)
                .map_err(|_| "role_file_read_failed".to_string())?,
            policy_revision: 1,
            policy_hash: policy_hash_for_view(&parsed.view, &parsed.view.purpose),
            policy_updated_at_ms: now,
            description_origin: TextOrigin::User,
            instructions_origin: TextOrigin::User,
        };
        let mut next_states = states;
        next_states.push(ManagedRoleState::new(record.clone()));
        if let Err(error) = commit_role_states(&state, next_states) {
            record_role_error(&mut audit, &error);
            return Err(error);
        }
        let detail = state_detail(
            &state.paths,
            ManagedRoleState::new(record),
            Some(&capabilities),
        )?;
        audit.success(1, 0, 1);
        Ok(detail)
    })();
    finish_role_command_result(&mut audit, result)
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_reorder(
    state: State<'_, AppState>,
    input: RoleReorderInput,
) -> Result<RoleListDto, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "role:reorder", None);
    let result = (|| {
        let requested = input
            .ids
            .iter()
            .map(|id| parse_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let _write = match state.guard_coordinator.try_guard_write() {
            Ok(write) => write,
            Err(error) => {
                record_role_error(&mut audit, &error);
                return Err(error);
            }
        };
        let states = load_role_states(&state.config_store)?;
        let original_orders = states
            .iter()
            .map(|managed| (managed.id().to_string(), managed.record.order))
            .collect::<std::collections::HashMap<_, _>>();
        let mut next_states = states;
        ManagedRoleState::reorder(&mut next_states, &requested)
            .map_err(|error| error.to_string())?;
        let changed = next_states
            .iter()
            .filter(|managed| {
                original_orders
                    .get(managed.id().as_str())
                    .is_some_and(|order| *order != managed.record.order)
            })
            .count()
            .min(u32::MAX as usize) as u32;
        let total = next_states.len().min(u32::MAX as usize) as u32;
        commit_role_states(&state, next_states)?;
        drop(_write);
        let result = role_list_impl(&state)?;
        audit.success(changed, total.saturating_sub(changed), 1);
        Ok(result)
    })();
    finish_role_command_result(&mut audit, result)
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_stop_managing(
    state: State<'_, AppState>,
    id: String,
) -> Result<RoleDiscoveryDto, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "role:stop_managing", Some(&id));
    let result = (|| {
        let id = parse_id(&id)?;
        validate_default_action(&id, RoleAction::StopManaging)
            .map_err(|error| error.to_string())?;
        let _write = match state.guard_coordinator.try_guard_write() {
            Ok(write) => write,
            Err(error) => {
                record_role_error(&mut audit, &error);
                return Err(error);
            }
        };
        let states = load_role_states(&state.config_store)?;
        if !states.iter().any(|candidate| candidate.id() == &id) {
            return Err("role_not_managed".to_string());
        }
        let remaining = states
            .iter()
            .filter(|candidate| candidate.id() != &id)
            .cloned()
            .collect::<Vec<_>>();
        let present_ids = role_ids_with_files(&state.paths, &remaining)?;
        let agents_before_bytes = read_agents_snapshot(&state.paths)?;
        let agents_before = document_from_snapshot(agents_before_bytes.as_deref())?;
        let agents_after = replace_role_directory_block(&agents_before, &remaining, &present_ids)?;
        let result = state.config_store.with_launcher_transaction(|launcher| {
            validate_role_states(&remaining).map_err(|error| error.to_string())?;
            launcher.config_mut().codex_guard.roles = remaining.clone();
            let codex_writer = PlatformAtomicFileWriter;
            let mut writes = vec![TransactionWrite {
                participant: JournalParticipant::Codex,
                relative_file: "AGENTS.md".to_string(),
                target: state.paths.codex_file("AGENTS.md"),
                original: agents_before_bytes.clone(),
                candidate_exists: true,
                candidate: agents_after.as_bytes().to_vec(),
                writer: &codex_writer,
            }];
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
        if let Err(error) = &result {
            record_role_error(&mut audit, error);
            if is_recovery_blocking_error(error) {
                state
                    .guard_coordinator
                    .mark_recovery_blocked("recovery_failed");
            }
            return Err(error.clone());
        }
        drop(_write);
        let result = role_discover_impl(&state)?;
        audit.success(1, 0, 1);
        Ok(result)
    })();
    finish_role_command_result(&mut audit, result)
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_delete(
    state: State<'_, AppState>,
    id: String,
) -> Result<RoleDeleteReport, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "role:delete", Some(&id));
    let result = (|| {
        let id = parse_id(&id)?;
        validate_default_action(&id, RoleAction::Delete).map_err(|error| error.to_string())?;
        let _write = match state.guard_coordinator.try_guard_write() {
            Ok(write) => write,
            Err(error) => {
                record_role_error(&mut audit, &error);
                return Err(error);
            }
        };
        let states = load_role_states(&state.config_store)?;
        if !states.iter().any(|candidate| candidate.id() == &id) {
            return Err("role_not_managed".to_string());
        }
        let remaining = states
            .iter()
            .filter(|candidate| candidate.id() != &id)
            .cloned()
            .collect::<Vec<_>>();
        let present_ids = role_ids_with_files(&state.paths, &remaining)?;
        let agents_before_bytes = read_agents_snapshot(&state.paths)?;
        let agents_before = document_from_snapshot(agents_before_bytes.as_deref())?;
        let agents_after = replace_role_directory_block(&agents_before, &remaining, &present_ids)?;
        let original_bytes = super::roles_store::read_role_file(&state.paths, &id)?;
        let removed = original_bytes.is_some();
        let result = state.config_store.with_launcher_transaction(|launcher| {
            validate_role_states(&remaining).map_err(|error| error.to_string())?;
            launcher.config_mut().codex_guard.roles = remaining.clone();
            let codex_writer = PlatformAtomicFileWriter;
            let mut writes = Vec::with_capacity(3);
            if original_bytes.is_some() {
                writes.push(TransactionWrite {
                    participant: JournalParticipant::Codex,
                    relative_file: super::roles::role_relative_path(&id),
                    target: role_target(&state.paths, &id)?,
                    original: original_bytes.clone(),
                    candidate_exists: false,
                    candidate: Vec::new(),
                    writer: &codex_writer,
                });
            }
            writes.push(TransactionWrite {
                participant: JournalParticipant::Codex,
                relative_file: "AGENTS.md".to_string(),
                target: state.paths.codex_file("AGENTS.md"),
                original: agents_before_bytes.clone(),
                candidate_exists: true,
                candidate: agents_after.as_bytes().to_vec(),
                writer: &codex_writer,
            });
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
        if let Err(error) = &result {
            record_role_error(&mut audit, error);
            if is_recovery_blocking_error(error) {
                state
                    .guard_coordinator
                    .mark_recovery_blocked("recovery_failed");
            }
            return Err(error.clone());
        }
        let report = RoleDeleteReport {
            id: id.to_string(),
            file_removed: removed,
            directory_updated: true,
            files_removed: u32::from(removed),
            backup_summary: removed.then(|| "files:1;backups:1".to_string()),
        };
        audit.success(1, 0, 1 + u32::from(removed));
        Ok(report)
    })();
    finish_role_command_result(&mut audit, result)
}

fn role_ids_with_files(
    paths: &AppPaths,
    states: &[ManagedRoleState],
) -> Result<HashSet<String>, String> {
    states
        .iter()
        .filter_map(|state| match role_file_present(paths, state.id()) {
            Ok(true) => Some(Ok(state.id().to_string())),
            Ok(false) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn read_agents_snapshot(paths: &AppPaths) -> Result<Option<Vec<u8>>, String> {
    let target = paths.codex_file("AGENTS.md");
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("agents_file_invalid".to_string())
        }
        Ok(_) => std::fs::read(target)
            .map(Some)
            .map_err(|_| "agents_file_read_failed".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("agents_file_read_failed".to_string()),
    }
}

fn document_from_snapshot(snapshot: Option<&[u8]>) -> Result<String, String> {
    snapshot
        .map(|bytes| {
            String::from_utf8(bytes.to_vec()).map_err(|_| "agents_file_read_failed".to_string())
        })
        .transpose()
        .map(|document| document.unwrap_or_default())
}

pub(crate) fn migration_choice(raw: &str) -> Result<RoleMigrationChoice, String> {
    match raw {
        "adopt_live" => Ok(RoleMigrationChoice::AdoptLive),
        "adopt_stored" => Ok(RoleMigrationChoice::AdoptStored),
        "use_safe_default" => Ok(RoleMigrationChoice::UseSafeDefault),
        _ => Err(RoleError::new(RoleErrorCode::InvalidMigrationChoice).to_string()),
    }
}

pub(crate) fn migration_result_dto(
    result: RoleMigrationResult,
    paths: &AppPaths,
) -> Result<RoleMigrationDto, String> {
    let state = ManagedRoleState {
        record: result.record,
        lifecycle: result.lifecycle,
    };
    Ok(RoleMigrationDto {
        status: "resolved".to_string(),
        reason: None,
        choices: Vec::new(),
        role: role_summaries(paths, &[state], None)?
            .into_iter()
            .next()
            .map(summary_dto),
    })
}

pub(crate) fn migration_plan_dto(plan: RoleMigrationPlan) -> RoleMigrationDto {
    match plan {
        RoleMigrationPlan::Auto { .. } => RoleMigrationDto {
            status: "auto".to_string(),
            reason: None,
            choices: Vec::new(),
            role: None,
        },
        RoleMigrationPlan::Pending { reason, choices } => RoleMigrationDto {
            status: "pending".to_string(),
            reason: Some(stable_enum(&reason)),
            choices: choices
                .into_iter()
                .map(|choice| stable_enum(&choice))
                .collect(),
            role: None,
        },
    }
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_capability_get(state: State<'_, AppState>) -> Result<CapabilitySnapshotDto, String> {
    capability_command_result(&state, false)
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_capability_refresh(
    state: State<'_, AppState>,
) -> Result<CapabilitySnapshotDto, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "capability:refresh", None);
    let result = (|| {
        let result = capability_command_result(&state, true)?;
        audit.success(0, result.models.len().min(u32::MAX as usize) as u32, 0);
        Ok(result)
    })();
    finish_role_command_result(&mut audit, result)
}

fn capability_snapshot_dto(
    snapshot: CapabilitySnapshot,
    available: bool,
    error: Option<String>,
) -> CapabilitySnapshotDto {
    CapabilitySnapshotDto {
        available,
        executable_identity: Some(snapshot.executable_identity),
        fetched_at_ms: Some(snapshot.fetched_at_ms),
        models: snapshot
            .models
            .into_iter()
            .map(|model| CapabilityModelDto {
                slug: model.slug,
                multi_agent_version: model.multi_agent_version,
                efforts: model.supported_reasoning_levels,
            })
            .collect(),
        error,
    }
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_migration_plan(
    state: State<'_, AppState>,
    safe_default_toml: String,
) -> Result<RoleMigrationDto, String> {
    let mut audit = OperationAuditGuard::new(
        &state.paths,
        "role:migration:plan",
        Some(super::roles::DEFAULT_ROLE_ID),
    );
    let result = (|| {
        let config = state.config_store.load_launcher()?;
        let legacy = config.codex_guard.params.get("agents-default-toml");
        let stored = legacy
            .and_then(|state| state.value.as_ref())
            .and_then(serde_json::Value::as_str);
        let default_id = RoleId::new(super::roles::DEFAULT_ROLE_ID).expect("valid default id");
        let live_bytes = super::roles_store::read_role_file(&state.paths, &default_id)?;
        let live = live_bytes
            .as_deref()
            .map(std::str::from_utf8)
            .transpose()
            .map_err(|_| RoleError::new(RoleErrorCode::InvalidText).to_string())?;
        let lifecycle = legacy
            .map(|state| {
                RoleLifecycle::from_flags(state.applied, state.locked)
                    .unwrap_or(RoleLifecycle::Disabled)
            })
            .unwrap_or(RoleLifecycle::Disabled);
        let result =
            super::roles_store::plan_default_migration(stored, live, lifecycle, &safe_default_toml)
                .map(migration_plan_dto)?;
        audit.success(0, 1, 0);
        Ok(result)
    })();
    finish_role_command_result(&mut audit, result)
}

#[tauri::command(async)]
#[specta::specta]
pub fn guard_role_migration_resolve(
    state: State<'_, AppState>,
    input: RoleMigrationResolveInput,
) -> Result<RoleMigrationDto, String> {
    let mut audit = OperationAuditGuard::new(
        &state.paths,
        "role:migration:resolve",
        Some(super::roles::DEFAULT_ROLE_ID),
    );
    let result = (|| {
        let choice = migration_choice(&input.choice)?;
        let _write = match state.guard_coordinator.try_guard_write() {
            Ok(write) => write,
            Err(error) => {
                record_role_error(&mut audit, &error);
                return Err(error);
            }
        };
        let default_id = RoleId::new(super::roles::DEFAULT_ROLE_ID).expect("valid default id");
        let live_bytes = super::roles_store::read_role_file(&state.paths, &default_id)?;
        let live = live_bytes
            .as_deref()
            .map(std::str::from_utf8)
            .transpose()
            .map_err(|_| RoleError::new(RoleErrorCode::InvalidText).to_string())?;
        let agents_before_bytes = read_agents_snapshot(&state.paths)?;
        let agents_before = document_from_snapshot(agents_before_bytes.as_deref())?;
        let result = state.config_store.with_launcher_transaction(|launcher| {
            let legacy = launcher
                .config()
                .codex_guard
                .params
                .get("agents-default-toml");
            let stored = legacy
                .and_then(|state| state.value.as_ref())
                .and_then(serde_json::Value::as_str);
            let lifecycle = legacy
                .map(|state| {
                    RoleLifecycle::from_flags(state.applied, state.locked)
                        .unwrap_or(RoleLifecycle::Disabled)
                })
                .unwrap_or(RoleLifecycle::Disabled);
            let result = super::roles_store::resolve_default_migration(
                choice,
                stored,
                live,
                &input.safe_default_toml,
                lifecycle,
                now_secs().saturating_mul(1000),
            )?;
            let should_write = live != Some(result.record.expected_toml.as_str());
            let next_state = ManagedRoleState {
                record: result.record.clone(),
                lifecycle: result.lifecycle,
            };
            let mut next_states = launcher.config().codex_guard.roles.clone();
            next_states.retain(|state| state.id() != &default_id);
            next_states.push(next_state);
            next_states.sort_by_key(|state| state.record.order);
            validate_role_states(&next_states).map_err(|error| error.to_string())?;

            let mut present_ids = role_ids_with_files(&state.paths, &next_states)?;
            // Migration always materializes the selected default role, even when
            // the live file already matched and therefore needs no replacement.
            present_ids.insert(default_id.to_string());
            let agents_after =
                replace_role_directory_block(&agents_before, &next_states, &present_ids)?;

            let mut writes = Vec::with_capacity(3);
            let codex_writer = PlatformAtomicFileWriter;
            if should_write {
                writes.push(TransactionWrite {
                    participant: JournalParticipant::Codex,
                    relative_file: super::roles::role_relative_path(&default_id),
                    target: role_target(&state.paths, &default_id)?,
                    original: live_bytes.clone(),
                    candidate_exists: true,
                    candidate: result.record.expected_toml.as_bytes().to_vec(),
                    writer: &codex_writer,
                });
            }
            writes.push(TransactionWrite {
                participant: JournalParticipant::Codex,
                relative_file: "AGENTS.md".to_string(),
                target: state.paths.codex_file("AGENTS.md"),
                original: agents_before_bytes.clone(),
                candidate_exists: true,
                candidate: agents_after.as_bytes().to_vec(),
                writer: &codex_writer,
            });
            let guard = &mut launcher.config_mut().codex_guard;
            guard.roles = next_states;
            guard.params.remove("agents-default-toml");
            writes.push(TransactionWrite {
                participant: JournalParticipant::Launcher,
                relative_file: "config.json".to_string(),
                target: launcher.target().to_path_buf(),
                original: launcher.original().map(ToOwned::to_owned),
                candidate_exists: true,
                candidate: launcher.candidate_bytes()?,
                writer: launcher.writer(),
            });
            execute_transaction_batch(&state.paths, writes).map(|()| (result, should_write))
        });
        if let Err(error) = &result {
            record_role_error(&mut audit, error);
            if is_recovery_blocking_error(error) {
                state
                    .guard_coordinator
                    .mark_recovery_blocked("recovery_failed");
            }
            return Err(error.clone());
        }
        let (result, role_file_written) = result.expect("checked successful role migration");
        drop(_write);
        let dto = migration_result_dto(result, &state.paths)?;
        audit.success(1, 0, 1 + u32::from(role_file_written));
        Ok(dto)
    })();
    finish_role_command_result(&mut audit, result)
}
