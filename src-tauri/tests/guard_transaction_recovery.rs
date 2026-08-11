#[allow(dead_code)]
#[path = "../src/codex_guard/atomic_store.rs"]
mod atomic_store;
#[allow(dead_code)]
#[path = "../src/codex_guard/journal.rs"]
mod journal;
#[allow(dead_code)]
#[path = "../src/codex_guard/transaction.rs"]
mod transaction;

mod codex_guard {
    pub(crate) use super::{journal, transaction};
}

use codex_guard::journal::{JournalEntry, JournalEnvelope, JournalParticipant};
use codex_guard::transaction::{
    recovery_action, RecoveryAction, TransactionPhase, TransactionState,
};

fn entry() -> JournalEntry {
    JournalEntry {
        participant: JournalParticipant::Codex,
        relative_file: "config.toml".into(),
        original_exists: true,
        original_sha256: "0".repeat(64),
        candidate_exists: true,
        candidate_sha256: "1".repeat(64),
        snapshot_ref: "snapshots/entry-0.bin".into(),
        completed: false,
        post_checked: false,
        restored: false,
    }
}

#[test]
fn recovery_never_cleans_an_uncommitted_journal() {
    let mut journal = JournalEnvelope::new("batch-1".into(), vec![entry()]);
    journal.phase = TransactionPhase::Completed;
    assert_eq!(recovery_action(&journal), RecoveryAction::RestorePending);

    journal.commit_marker = true;
    assert_eq!(recovery_action(&journal), RecoveryAction::CleanupCompleted);
}

#[test]
fn transaction_state_requires_the_durable_phase_order() {
    let mut state = TransactionState::new();
    assert!(state.transition(TransactionPhase::Snapshot).is_ok());
    assert!(state.transition(TransactionPhase::Writing).is_ok());
    assert!(state.transition(TransactionPhase::PostCheck).is_ok());
    assert!(state.transition(TransactionPhase::Completed).is_ok());
    assert!(state.transition(TransactionPhase::Restoring).is_err());
}

#[test]
fn journal_rejects_invalid_participant_target_and_absolute_references() {
    let mut journal = JournalEnvelope::new("batch-1".into(), vec![entry()]);
    journal.entries[0].participant = JournalParticipant::Launcher;
    assert!(journal.validate().is_err());

    journal.entries[0].participant = JournalParticipant::Codex;
    journal.entries[0].snapshot_ref = "/tmp/snapshot".into();
    assert!(journal.validate().is_err());
}
