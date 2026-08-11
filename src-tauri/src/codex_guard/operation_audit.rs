//! Minimal local operation-audit storage.
//!
//! The operation log is deliberately separate from transaction journals.  It contains only
//! the allow-listed DTO from `audit.rs`, is rewritten atomically after retention, and never
//! exposes the source file path or arbitrary error text.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::atomic_store::{AtomicFileWriter, PlatformAtomicFileWriter};
use super::audit::{
    operation_audit_jsonl, retain_operation_audit, sanitize_operation_audit, OperationAuditInput,
    OperationAuditPhase, OperationAuditRecord, OperationAuditResult, AUDIT_SCHEMA_VERSION,
    MAX_OPERATION_AUDIT_ENTRIES,
};
use super::AppPaths;

const OPERATION_AUDIT_FILE: &str = "codex-guard-operation-audit.jsonl";
const OPERATION_AUDIT_ERROR_FILE: &str = "codex-guard-operation-audit-error.json";
const MAX_AUDIT_FILE_BYTES: usize = MAX_OPERATION_AUDIT_ENTRIES * 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationAuditStoreError {
    Read,
    InvalidRecord,
    TooLarge,
    Serialize,
    Write,
}

impl OperationAuditStoreError {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Read => "operation_audit_read_failed",
            Self::InvalidRecord => "operation_audit_invalid_record",
            Self::TooLarge => "operation_audit_too_large",
            Self::Serialize => "operation_audit_serialize_failed",
            Self::Write => "operation_audit_write_failed",
        }
    }
}

static STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn lock() -> Result<std::sync::MutexGuard<'static, ()>, OperationAuditStoreError> {
    STORE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| OperationAuditStoreError::Write)
}

pub(crate) fn operation_audit_path(paths: &AppPaths) -> PathBuf {
    paths.launcher_root().join(OPERATION_AUDIT_FILE)
}

fn operation_audit_error_path(paths: &AppPaths) -> PathBuf {
    paths.launcher_root().join(OPERATION_AUDIT_ERROR_FILE)
}

fn append_error_record(paths: &AppPaths, now_ms: u64) {
    // Keep the failure visible through the existing audit-list command without echoing the
    // underlying filesystem error.  This is a best-effort sidecar: a failed sidecar write is
    // still surfaced through the original stable store error.
    let input = OperationAuditInput {
        at_ms: now_ms,
        batch_id: "operation-audit".to_string(),
        scope: "system".to_string(),
        relative_file: None,
        phase: OperationAuditPhase::Audit,
        result: OperationAuditResult::Failed,
        error_code: Some("operation_audit_append_failed".to_string()),
        changed: 0,
        unchanged: 0,
        files: 0,
        role_id: None,
        model: None,
        effort: None,
    };
    let Ok(record) = sanitize_operation_audit(&input) else {
        return;
    };
    let Ok(bytes) = serde_json::to_vec(&record) else {
        return;
    };
    let _ = PlatformAtomicFileWriter.replace(&operation_audit_error_path(paths), &bytes);
}

fn clear_error_record(paths: &AppPaths) {
    let _ = fs::remove_file(operation_audit_error_path(paths));
}

/// Append a command-level audit row using only stable identifiers.  Callers intentionally ignore
/// the returned store error: a committed operation must not be rolled back because its auxiliary
/// audit row failed, while the sidecar makes that failure visible to the audit-list command.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_operation_audit(
    paths: &AppPaths,
    scope: impl Into<String>,
    phase: OperationAuditPhase,
    result: OperationAuditResult,
    error_code: Option<&str>,
    changed: u32,
    unchanged: u32,
    files: u32,
    role_id: Option<&str>,
) {
    let at_ms = super::now_secs().saturating_mul(1_000);
    let input = OperationAuditInput {
        at_ms,
        batch_id: super::journal::new_batch_id(),
        scope: scope.into(),
        relative_file: None,
        phase,
        result,
        error_code: error_code.map(str::to_string),
        changed,
        unchanged,
        files,
        role_id: role_id.map(str::to_string),
        model: None,
        effort: None,
    };
    if let Err(error) = append_operation_audit(paths, &input, at_ms) {
        log::error!("guard operation audit append failed: {}", error.code());
    }
}

/// Scope guard used by command boundaries.  It records failures from `?`/early returns as well
/// as successful command completion, so an operation cannot disappear from the local history.
pub(crate) struct OperationAuditGuard<'a> {
    paths: &'a AppPaths,
    scope: String,
    role_id: Option<String>,
    result: Option<(OperationAuditResult, Option<String>, u32, u32, u32)>,
}

impl<'a> OperationAuditGuard<'a> {
    pub(crate) fn new(
        paths: &'a AppPaths,
        scope: impl Into<String>,
        role_id: Option<&str>,
    ) -> Self {
        Self {
            paths,
            scope: scope.into(),
            role_id: role_id.map(str::to_string),
            result: None,
        }
    }

    /// Record an explicit operation result before the guard is dropped.
    ///
    /// Error codes are accepted only as stable identifiers by the audit sanitizer.  The result
    /// and counts are kept until `Drop`, so callers can use this at the exact boundary where an
    /// operation returns an early error without changing the command's return value.
    pub(crate) fn record(
        &mut self,
        result: OperationAuditResult,
        error_code: Option<&str>,
        changed: u32,
        unchanged: u32,
        files: u32,
    ) {
        self.result = Some((
            result,
            error_code.map(str::to_string),
            changed,
            unchanged,
            files,
        ));
    }

    pub(crate) fn success(&mut self, changed: u32, unchanged: u32, files: u32) {
        self.record(
            OperationAuditResult::Success,
            None,
            changed,
            unchanged,
            files,
        );
    }

    pub(crate) fn failure(
        &mut self,
        error_code: impl Into<String>,
        changed: u32,
        unchanged: u32,
        files: u32,
    ) {
        self.record_owned(
            OperationAuditResult::Failed,
            Some(error_code.into()),
            changed,
            unchanged,
            files,
        );
    }

    pub(crate) fn rejected(
        &mut self,
        error_code: impl Into<String>,
        changed: u32,
        unchanged: u32,
        files: u32,
    ) {
        self.record_owned(
            OperationAuditResult::Rejected,
            Some(error_code.into()),
            changed,
            unchanged,
            files,
        );
    }

    pub(crate) fn busy(
        &mut self,
        error_code: impl Into<String>,
        changed: u32,
        unchanged: u32,
        files: u32,
    ) {
        self.record_owned(
            OperationAuditResult::Busy,
            Some(error_code.into()),
            changed,
            unchanged,
            files,
        );
    }

    pub(crate) fn rolled_back(
        &mut self,
        error_code: impl Into<String>,
        changed: u32,
        unchanged: u32,
        files: u32,
    ) {
        self.record_owned(
            OperationAuditResult::RolledBack,
            Some(error_code.into()),
            changed,
            unchanged,
            files,
        );
    }

    pub(crate) fn critical(
        &mut self,
        error_code: impl Into<String>,
        changed: u32,
        unchanged: u32,
        files: u32,
    ) {
        self.record_owned(
            OperationAuditResult::Critical,
            Some(error_code.into()),
            changed,
            unchanged,
            files,
        );
    }

    fn record_owned(
        &mut self,
        result: OperationAuditResult,
        error_code: Option<String>,
        changed: u32,
        unchanged: u32,
        files: u32,
    ) {
        self.result = Some((result, error_code, changed, unchanged, files));
    }
}

impl Drop for OperationAuditGuard<'_> {
    fn drop(&mut self) {
        let (result, error_code, changed, unchanged, files) =
            self.result.take().unwrap_or_else(|| {
                (
                    OperationAuditResult::Failed,
                    Some("operation_failed".to_string()),
                    0,
                    0,
                    0,
                )
            });
        let phase = match result {
            OperationAuditResult::Success => OperationAuditPhase::Completed,
            OperationAuditResult::RolledBack | OperationAuditResult::Critical => {
                OperationAuditPhase::Recovery
            }
            OperationAuditResult::Rejected | OperationAuditResult::Busy => {
                OperationAuditPhase::Preflight
            }
            OperationAuditResult::Failed => OperationAuditPhase::Verify,
        };
        record_operation_audit(
            self.paths,
            self.scope.clone(),
            phase,
            result,
            error_code.as_deref(),
            changed,
            unchanged,
            files,
            self.role_id.as_deref(),
        );
    }
}

fn read_error_record(
    paths: &AppPaths,
) -> Result<Option<OperationAuditRecord>, OperationAuditStoreError> {
    let path = operation_audit_error_path(paths);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(OperationAuditStoreError::Read),
    };
    if bytes.len() > 8 * 1024 {
        return Err(OperationAuditStoreError::TooLarge);
    }
    let record = serde_json::from_slice::<OperationAuditRecord>(&bytes)
        .map_err(|_| OperationAuditStoreError::InvalidRecord)?;
    if record.schema_version != AUDIT_SCHEMA_VERSION {
        return Err(OperationAuditStoreError::InvalidRecord);
    }
    Ok(Some(record))
}

pub(crate) fn read_operation_audit(
    paths: &AppPaths,
) -> Result<Vec<OperationAuditRecord>, OperationAuditStoreError> {
    let path = operation_audit_path(paths);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(OperationAuditStoreError::Read),
    };
    if bytes.len() > MAX_AUDIT_FILE_BYTES {
        return Err(OperationAuditStoreError::TooLarge);
    }
    let mut records = Vec::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let record: OperationAuditRecord =
            serde_json::from_slice(line).map_err(|_| OperationAuditStoreError::InvalidRecord)?;
        if record.schema_version != AUDIT_SCHEMA_VERSION {
            return Err(OperationAuditStoreError::InvalidRecord);
        }
        records.push(record);
        if records.len() > MAX_OPERATION_AUDIT_ENTRIES {
            return Err(OperationAuditStoreError::TooLarge);
        }
    }
    Ok(records)
}

fn write_records(
    path: &Path,
    records: &[OperationAuditRecord],
) -> Result<(), OperationAuditStoreError> {
    let bytes = operation_audit_jsonl(records).map_err(|_| OperationAuditStoreError::Serialize)?;
    if bytes.len() > MAX_AUDIT_FILE_BYTES {
        return Err(OperationAuditStoreError::TooLarge);
    }
    PlatformAtomicFileWriter
        .replace(path, &bytes)
        .map_err(|_| OperationAuditStoreError::Write)
}

/// Append one sanitized operation record and prune it before the atomic replace.
pub(crate) fn append_operation_audit(
    paths: &AppPaths,
    input: &OperationAuditInput,
    now_ms: u64,
) -> Result<OperationAuditRecord, OperationAuditStoreError> {
    let _guard = lock()?;
    let record = match sanitize_operation_audit(input) {
        Ok(record) => record,
        Err(_) => {
            append_error_record(paths, now_ms);
            return Err(OperationAuditStoreError::InvalidRecord);
        }
    };
    let mut records = match read_operation_audit(paths) {
        Ok(records) => records,
        Err(error) => {
            append_error_record(paths, now_ms);
            return Err(error);
        }
    };
    records.push(record.clone());
    let retained = retain_operation_audit(&records, now_ms);
    if let Err(error) = write_records(&operation_audit_path(paths), &retained) {
        append_error_record(paths, now_ms);
        return Err(error);
    }
    clear_error_record(paths);
    Ok(record)
}

pub(crate) fn list_operation_audit(
    paths: &AppPaths,
    now_ms: u64,
) -> Result<Vec<OperationAuditRecord>, OperationAuditStoreError> {
    let _guard = lock()?;
    let records = read_operation_audit(paths)?;
    let mut records = retain_operation_audit(&records, now_ms);
    if let Some(error) = read_error_record(paths)? {
        records.push(error);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::audit::{
        OperationAuditPhase, OperationAuditResult, OPERATION_AUDIT_RETENTION_MS,
    };
    use tempfile::tempdir;

    fn input(at_ms: u64) -> OperationAuditInput {
        OperationAuditInput {
            at_ms,
            batch_id: "batch-1".to_string(),
            scope: "global".to_string(),
            relative_file: Some("config.toml".to_string()),
            phase: OperationAuditPhase::Completed,
            result: OperationAuditResult::Success,
            error_code: None,
            changed: 1,
            unchanged: 0,
            files: 1,
            role_id: Some("default".to_string()),
            model: Some("gpt-test".to_string()),
            effort: Some("high".to_string()),
        }
    }

    #[test]
    fn append_is_atomic_and_prunes_old_records() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let old = input(1);
        append_operation_audit(&paths, &old, OPERATION_AUDIT_RETENTION_MS + 2).unwrap();
        append_operation_audit(
            &paths,
            &input(OPERATION_AUDIT_RETENTION_MS + 2),
            OPERATION_AUDIT_RETENTION_MS + 2,
        )
        .unwrap();
        let records = list_operation_audit(&paths, OPERATION_AUDIT_RETENTION_MS + 2).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].at_ms, OPERATION_AUDIT_RETENTION_MS + 2);
        assert!(operation_audit_path(&paths).exists());
    }

    #[test]
    fn malformed_rows_fail_closed() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let path = operation_audit_path(&paths);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"{not-json}\n").unwrap();
        assert_eq!(
            read_operation_audit(&paths).unwrap_err(),
            OperationAuditStoreError::InvalidRecord
        );
    }

    #[test]
    fn append_failure_leaves_a_stable_sidecar_record() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let path = operation_audit_path(&paths);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"{not-json}\n").unwrap();

        let error = append_operation_audit(&paths, &input(10), 10).unwrap_err();
        assert_eq!(error, OperationAuditStoreError::InvalidRecord);

        let sidecar = read_error_record(&paths).unwrap().expect("error sidecar");
        assert_eq!(sidecar.result, OperationAuditResult::Failed);
        assert_eq!(
            sidecar.error_code.as_deref(),
            Some("operation_audit_append_failed")
        );
        assert_eq!(sidecar.scope, "system");
    }

    #[test]
    fn guard_records_explicit_results_codes_and_counts() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());

        {
            let mut guard = OperationAuditGuard::new(&paths, "failed", None);
            guard.failure("config_read_failed", 1, 2, 3);
        }
        {
            let mut guard = OperationAuditGuard::new(&paths, "rejected", None);
            guard.rejected("guard_contract_version_unsupported", 4, 5, 6);
        }
        {
            let mut guard = OperationAuditGuard::new(&paths, "busy", None);
            guard.busy("guard_busy", 7, 8, 9);
        }
        {
            let mut guard = OperationAuditGuard::new(&paths, "rolled-back", None);
            guard.rolled_back("guard_transaction_rolled_back", 10, 11, 12);
        }
        {
            let mut guard = OperationAuditGuard::new(&paths, "critical", None);
            guard.critical("restore_failed_critical", 13, 14, 15);
        }
        {
            let mut guard = OperationAuditGuard::new(&paths, "recorded", None);
            guard.record(
                OperationAuditResult::Rejected,
                Some("recorded_rejection"),
                16,
                17,
                18,
            );
        }

        let records = read_operation_audit(&paths).expect("operation records");
        assert_eq!(records.len(), 6);
        let expected = [
            (
                "failed",
                OperationAuditResult::Failed,
                OperationAuditPhase::Verify,
                "config_read_failed",
                (1, 2, 3),
            ),
            (
                "rejected",
                OperationAuditResult::Rejected,
                OperationAuditPhase::Preflight,
                "guard_contract_version_unsupported",
                (4, 5, 6),
            ),
            (
                "busy",
                OperationAuditResult::Busy,
                OperationAuditPhase::Preflight,
                "guard_busy",
                (7, 8, 9),
            ),
            (
                "rolled-back",
                OperationAuditResult::RolledBack,
                OperationAuditPhase::Recovery,
                "guard_transaction_rolled_back",
                (10, 11, 12),
            ),
            (
                "critical",
                OperationAuditResult::Critical,
                OperationAuditPhase::Recovery,
                "restore_failed_critical",
                (13, 14, 15),
            ),
            (
                "recorded",
                OperationAuditResult::Rejected,
                OperationAuditPhase::Preflight,
                "recorded_rejection",
                (16, 17, 18),
            ),
        ];
        for (record, (scope, result, phase, error_code, counts)) in records.iter().zip(expected) {
            assert_eq!(record.scope, scope);
            assert_eq!(record.result, result);
            assert_eq!(record.phase, phase);
            assert_eq!(record.error_code.as_deref(), Some(error_code));
            assert_eq!((record.changed, record.unchanged, record.files), counts);
        }
    }

    #[test]
    fn guard_drop_uses_a_stable_default_failure_code() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let _guard = OperationAuditGuard::new(&paths, "implicit-failure", None);
        drop(_guard);

        let records = read_operation_audit(&paths).expect("operation record");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].result, OperationAuditResult::Failed);
        assert_eq!(records[0].error_code.as_deref(), Some("operation_failed"));
        assert_eq!(records[0].phase, OperationAuditPhase::Verify);
        assert_eq!(
            (records[0].changed, records[0].unchanged, records[0].files),
            (0, 0, 0)
        );
    }
}
