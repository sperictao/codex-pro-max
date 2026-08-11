//! Public IPC boundaries for runtime and operation audits.
//!
//! The command layer only assembles immutable role-policy snapshots and delegates all parsing,
//! redaction, retention, and bounded filesystem work to the audit modules.

use tauri::State;

use crate::AppState;

use super::audit::{ManagedRoleSnapshot, OperationAuditRecord, SubagentAuditResult};
use super::audit_runner::run_subagent_audit;
use super::now_secs;
use super::operation_audit::{list_operation_audit, OperationAuditGuard};
use super::roles_store::load_role_states;

fn now_ms() -> u64 {
    now_secs().saturating_mul(1_000)
}

fn role_snapshots(state: &AppState) -> Result<Vec<ManagedRoleSnapshot>, String> {
    load_role_states(&state.config_store)?
        .into_iter()
        .map(|role| {
            let view = role.record.view().map_err(|error| error.to_string())?;
            Ok(ManagedRoleSnapshot {
                role_id: role.record.id.to_string(),
                policy_revision: Some(role.record.policy_revision),
                policy_hash: Some(role.record.policy_hash),
                expected_model: Some(view.model),
                expected_effort: Some(view.effort),
                policy_updated_at_ms: Some(role.record.policy_updated_at_ms),
                managed: true,
            })
        })
        .collect()
}

fn stable_audit_error_code(error: &str) -> String {
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
        "audit_failed".to_string()
    }
}

fn record_audit_error(audit: &mut OperationAuditGuard<'_>, error: &str) {
    let code = stable_audit_error_code(error);
    if error == "audit_already_running" {
        audit.rejected(code, 0, 0, 0);
    } else if error.starts_with("guard transaction failed: ") {
        audit.rolled_back(code, 0, 0, 0);
    } else {
        audit.failure(code, 0, 0, 0);
    }
}

fn finish_audit_command_result<T>(
    audit: &mut OperationAuditGuard<'_>,
    result: Result<T, String>,
) -> Result<T, String> {
    if let Err(error) = &result {
        record_audit_error(audit, error);
    }
    result
}

/// Run the bounded runtime audit against the current Codex rollout and sampling stores.
#[tauri::command]
#[specta::specta]
pub fn guard_run_subagent_audit(state: State<'_, AppState>) -> Result<SubagentAuditResult, String> {
    let mut audit = OperationAuditGuard::new(&state.paths, "audit:runtime", None);
    let result = (|| {
        let roles = role_snapshots(&state)?;
        let result = run_subagent_audit(state.paths.codex_root(), &roles, now_ms())?;
        audit.success(0, result.agents.len().min(u32::MAX as usize) as u32, 0);
        Ok(result)
    })();
    finish_audit_command_result(&mut audit, result)
}

/// Return retained local operation-audit records. The store owns parsing and retention.
#[tauri::command]
#[specta::specta]
pub fn guard_operation_audit_list(
    state: State<'_, AppState>,
) -> Result<Vec<OperationAuditRecord>, String> {
    // Listing history is intentionally read-only. Recording this query would make
    // every refresh mutate the history it is displaying.
    list_operation_audit(&state.paths, now_ms()).map_err(|error| error.code().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::audit::OperationAuditResult;
    use crate::codex_guard::operation_audit::{operation_audit_path, read_operation_audit};

    #[test]
    fn audit_errors_use_stable_codes_and_phases() {
        let temp = tempfile::tempdir().unwrap();
        let paths = super::super::AppPaths::for_test(temp.path());
        {
            let mut audit = OperationAuditGuard::new(&paths, "audit:test", None);
            record_audit_error(&mut audit, "audit_already_running");
        }
        {
            let mut audit = OperationAuditGuard::new(&paths, "audit:test", None);
            record_audit_error(&mut audit, "filesystem details are not persisted");
        }
        let records = read_operation_audit(&paths).unwrap();
        assert_eq!(records[0].result, OperationAuditResult::Rejected);
        assert_eq!(
            records[0].error_code.as_deref(),
            Some("audit_already_running")
        );
        assert_eq!(records[1].result, OperationAuditResult::Failed);
        assert_eq!(records[1].error_code.as_deref(), Some("audit_failed"));
    }

    #[test]
    fn operation_audit_listing_does_not_append_history() {
        let temp = tempfile::tempdir().unwrap();
        let paths = super::super::AppPaths::for_test(temp.path());
        {
            let mut audit = OperationAuditGuard::new(&paths, "audit:test", None);
            audit.success(0, 1, 0);
        }

        let before = std::fs::read(operation_audit_path(&paths)).unwrap();
        let first = list_operation_audit(&paths, now_ms()).unwrap();
        let second = list_operation_audit(&paths, now_ms()).unwrap();
        let after = std::fs::read(operation_audit_path(&paths)).unwrap();

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(after, before);
    }
}
