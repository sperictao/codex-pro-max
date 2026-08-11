//! 事务阶段与恢复决策的纯领域模型。
//!
//! 这里不做文件 I/O，也不持有 Guard/ConfigStore 锁。持久化 envelope 见
//! `journal.rs`；把阶段约束留在纯模型中，避免执行器在失败分支里自行拼接状态。

use serde::{Deserialize, Serialize};

use super::journal::JournalEnvelope;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransactionPhase {
    Preflight,
    Snapshot,
    Writing,
    PostCheck,
    Restoring,
    Completed,
    Critical,
}

impl TransactionPhase {
    pub(crate) fn can_transition(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Preflight, Self::Snapshot)
                | (Self::Snapshot, Self::Writing)
                | (Self::Writing, Self::PostCheck)
                | (Self::Writing, Self::Restoring)
                | (Self::PostCheck, Self::Completed)
                | (Self::PostCheck, Self::Restoring)
                | (Self::Restoring, Self::Completed)
                | (Self::Restoring, Self::Critical)
                | (Self::Critical, Self::Restoring)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransactionErrorCode {
    InvalidTransition,
    JournalSchema,
    JournalIo,
    ReadFailed,
    IdentityChanged,
    ReplaceFailed,
    DurableBackupFailed,
    PostCheckFailed,
    RestoreFailedCritical,
    FaultInjected,
}

impl TransactionErrorCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidTransition => "invalid_transition",
            Self::JournalSchema => "journal_schema",
            Self::JournalIo => "journal_io",
            Self::ReadFailed => "read_failed",
            Self::IdentityChanged => "identity_changed",
            Self::ReplaceFailed => "replace_failed",
            Self::DurableBackupFailed => "durable_backup_failed",
            Self::PostCheckFailed => "post_check_failed",
            Self::RestoreFailedCritical => "restore_failed_critical",
            Self::FaultInjected => "fault_injected",
        }
    }
}

/// Test-only failure locations for the durable transaction protocol.
///
/// Keeping the points in the transaction domain makes it possible to exercise the
/// same production executor without replacing the filesystem with a mock success
/// path. A plan contains one failure, which keeps each test deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransactionFaultPoint {
    JournalCreate,
    JournalSync,
    Snapshot(usize),
    PreWriteIdentity(usize),
    DurableBackup(usize),
    Write(usize),
    Replace(usize),
    PostCheck(usize),
    StateSaveBefore(TransactionPhase),
    StateSaveAfter(TransactionPhase),
    CommitMarkerBefore,
    CommitMarkerAfter,
    Restore(usize),
    JournalCleanup,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TransactionFaultPlan {
    point: Option<TransactionFaultPoint>,
    tripped: bool,
}

impl TransactionFaultPlan {
    pub(crate) const fn disabled() -> Self {
        Self {
            point: None,
            tripped: false,
        }
    }

    #[cfg(test)]
    pub(crate) const fn fail_at(point: TransactionFaultPoint) -> Self {
        Self {
            point: Some(point),
            tripped: false,
        }
    }

    pub(crate) fn check(&mut self, point: TransactionFaultPoint) -> Result<(), TransactionError> {
        if !self.tripped && self.point == Some(point) {
            self.tripped = true;
            return Err(TransactionError::new(TransactionErrorCode::FaultInjected));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransactionError {
    pub(crate) code: TransactionErrorCode,
}

impl TransactionError {
    pub(crate) const fn new(code: TransactionErrorCode) -> Self {
        Self { code }
    }
}

/// Returns whether an operation left durable recovery state that must block the
/// next Guard write. A transaction that restored every participant successfully
/// is a rollback, not a recovery failure.
// Tauri command proc-macro entrypoints keep the callers reachable at runtime,
// but rustc's binary dead-code analysis cannot see that generated path.
#[allow(dead_code)]
pub(crate) fn is_recovery_blocking_error(error: &str) -> bool {
    error == "recovery_blocked"
        || error == "recovery_failed"
        || error == "migration_failed"
        || error.contains("restore_failed_critical")
        || error.contains("journal_")
        || error.contains("invalid_transition")
}

/// Converts a bounded transaction error into a stable audit code without
/// exposing arbitrary filesystem or parser text.
#[allow(dead_code)]
pub(crate) fn stable_transaction_error_code(error: &str) -> String {
    let candidate = error
        .strip_prefix("guard transaction failed: ")
        .unwrap_or(error);
    if !candidate.is_empty()
        && candidate.len() <= 96
        && candidate
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        candidate.to_string()
    } else {
        "operation_failed".to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryAction {
    CleanupCompleted,
    RestorePending,
}

pub(crate) fn recovery_action(journal: &JournalEnvelope) -> RecoveryAction {
    if journal.phase == TransactionPhase::Completed && journal.commit_marker {
        return RecoveryAction::CleanupCompleted;
    }
    RecoveryAction::RestorePending
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransactionState {
    pub(crate) phase: TransactionPhase,
}

impl TransactionState {
    pub(crate) const fn new() -> Self {
        Self {
            phase: TransactionPhase::Preflight,
        }
    }

    pub(crate) fn transition(&mut self, next: TransactionPhase) -> Result<(), TransactionError> {
        if !self.phase.can_transition(next) {
            return Err(TransactionError::new(
                TransactionErrorCode::InvalidTransition,
            ));
        }
        self.phase = next;
        Ok(())
    }

    pub(crate) fn mark_critical(&mut self) -> Result<(), TransactionError> {
        if self.phase == TransactionPhase::Completed {
            return Err(TransactionError::new(
                TransactionErrorCode::InvalidTransition,
            ));
        }
        self.phase = TransactionPhase::Critical;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::journal::{JournalEntry, JournalParticipant, JOURNAL_SCHEMA_VERSION};

    #[test]
    fn transaction_phase_rejects_skips_and_repeated_terminal_transitions() {
        let mut state = TransactionState::new();
        assert!(state.transition(TransactionPhase::Snapshot).is_ok());
        assert!(state.transition(TransactionPhase::Writing).is_ok());
        assert!(state.transition(TransactionPhase::Completed).is_err());
        assert!(state.transition(TransactionPhase::PostCheck).is_ok());
        assert!(state.transition(TransactionPhase::Completed).is_ok());
        assert!(state.transition(TransactionPhase::Completed).is_err());
    }

    #[test]
    fn recovery_action_requires_commit_marker_for_cleanup() {
        let mut journal = JournalEnvelope::new("batch-1".into(), vec![entry()]);
        journal.phase = TransactionPhase::Completed;
        assert_eq!(recovery_action(&journal), RecoveryAction::RestorePending);
        journal.commit_marker = true;
        assert_eq!(recovery_action(&journal), RecoveryAction::CleanupCompleted);
        journal.critical = true;
        assert_eq!(recovery_action(&journal), RecoveryAction::CleanupCompleted);
    }

    fn entry() -> JournalEntry {
        JournalEntry {
            participant: JournalParticipant::Codex,
            relative_file: "config.toml".into(),
            original_exists: true,
            original_sha256: "0".repeat(64),
            candidate_exists: true,
            candidate_sha256: "1".repeat(64),
            snapshot_ref: "snapshots/snapshot-0".into(),
            completed: false,
            post_checked: false,
            restored: false,
        }
    }

    #[test]
    fn journal_schema_version_is_stable() {
        assert_eq!(JOURNAL_SCHEMA_VERSION, 1);
    }

    #[test]
    fn critical_mark_is_fail_closed_after_completion() {
        let mut state = TransactionState::new();
        state.transition(TransactionPhase::Snapshot).unwrap();
        state.mark_critical().unwrap();
        assert_eq!(state.phase, TransactionPhase::Critical);
        assert!(state.mark_critical().is_ok());
        state.phase = TransactionPhase::Completed;
        assert_eq!(
            state.mark_critical().unwrap_err().code,
            TransactionErrorCode::InvalidTransition
        );
    }
}
