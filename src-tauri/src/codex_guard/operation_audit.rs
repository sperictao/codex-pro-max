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
/// Appends stay O(1) until the file crosses this size; crossing it triggers one read +
/// retain + atomic rewrite. Retention still runs on every list, so the cap only bounds
/// on-disk growth between compactions.
const COMPACT_AFTER_BYTES: u64 = 256 * 1024;

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
    let terminated = bytes.last() == Some(&b'\n');
    let mut records = Vec::new();
    let mut lines = bytes.split(|byte| *byte == b'\n').peekable();
    while let Some(line) = lines.next() {
        if line.is_empty() {
            continue;
        }
        if !terminated && lines.peek().is_none() {
            // A crash can tear the final append. The incomplete tail row is dropped; a
            // terminated-but-invalid row still fails closed below.
            break;
        }
        let record: OperationAuditRecord =
            serde_json::from_slice(line).map_err(|_| OperationAuditStoreError::InvalidRecord)?;
        if record.schema_version != AUDIT_SCHEMA_VERSION {
            return Err(OperationAuditStoreError::InvalidRecord);
        }
        records.push(record);
    }
    // Append-only writes can exceed the entry cap between compactions; reads bound memory
    // by keeping the newest records, matching the retention policy.
    if records.len() > MAX_OPERATION_AUDIT_ENTRIES {
        records.drain(..records.len() - MAX_OPERATION_AUDIT_ENTRIES);
    }
    Ok(records)
}

/// Append one pre-serialized line without rewriting history. A crash can tear the previous
/// append's final row; the tail is truncated back to the last complete line before writing,
/// so the new record never merges into a half-written one. The line then lands in a single
/// `write_all` followed by `sync_data`, and a newly created file also gets the directory
/// fsynced so the file name itself is durable.
fn append_audit_line(paths: &AppPaths, line: &[u8]) -> Result<(), OperationAuditStoreError> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let path = operation_audit_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| OperationAuditStoreError::Write)?;
    }
    let existed = fs::metadata(&path).is_ok();
    let mut file = fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&path)
        .map_err(|_| OperationAuditStoreError::Write)?;
    let len = file
        .metadata()
        .map_err(|_| OperationAuditStoreError::Write)?
        .len();
    if len > 0 {
        let scan = len.min(64 * 1024);
        let mut tail = vec![0u8; scan as usize];
        file.seek(SeekFrom::End(-(scan as i64)))
            .and_then(|_| file.read_exact(&mut tail))
            .map_err(|_| OperationAuditStoreError::Write)?;
        if tail.last() != Some(&b'\n') {
            let cut = match tail.iter().rposition(|byte| *byte == b'\n') {
                Some(position) => len - scan + position as u64 + 1,
                // 整个小文件都是撕裂残片：清空重来
                None if len == scan => 0,
                // 尾部窗口之外可能还有完整行：不猜边界，失败关闭交给 sidecar 暴露
                None => return Err(OperationAuditStoreError::Write),
            };
            file.set_len(cut)
                .map_err(|_| OperationAuditStoreError::Write)?;
        }
    }
    file.write_all(line)
        .and_then(|()| file.sync_data())
        .map_err(|_| OperationAuditStoreError::Write)?;
    if !existed {
        if let Some(parent) = path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
    }
    Ok(())
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

/// Append one sanitized operation record. Small files take the O(1) line-append path; past
/// the compaction threshold the file is read, pruned, and atomically rewritten.
pub(crate) fn append_operation_audit(
    paths: &AppPaths,
    input: &OperationAuditInput,
    now_ms: u64,
) -> Result<OperationAuditRecord, OperationAuditStoreError> {
    append_operation_audit_with_limit(paths, input, now_ms, COMPACT_AFTER_BYTES)
}

fn append_operation_audit_with_limit(
    paths: &AppPaths,
    input: &OperationAuditInput,
    now_ms: u64,
    compact_after_bytes: u64,
) -> Result<OperationAuditRecord, OperationAuditStoreError> {
    let _guard = lock()?;
    let record = match sanitize_operation_audit(input) {
        Ok(record) => record,
        Err(_) => {
            append_error_record(paths, now_ms);
            return Err(OperationAuditStoreError::InvalidRecord);
        }
    };
    let mut line = match serde_json::to_vec(&record) {
        Ok(line) => line,
        Err(_) => {
            append_error_record(paths, now_ms);
            return Err(OperationAuditStoreError::Serialize);
        }
    };
    line.push(b'\n');
    let size = fs::metadata(operation_audit_path(paths))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if size.saturating_add(line.len() as u64) <= compact_after_bytes {
        return match append_audit_line(paths, &line) {
            Ok(()) => {
                clear_error_record(paths);
                Ok(record)
            }
            Err(error) => {
                append_error_record(paths, now_ms);
                Err(error)
            }
        };
    }
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
    fn append_only_writes_concatenated_lines_without_rewriting_history() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        append_operation_audit(&paths, &input(1), 1).unwrap();
        append_operation_audit(&paths, &input(2), 2).unwrap();
        let raw = std::fs::read_to_string(operation_audit_path(&paths)).unwrap();
        assert_eq!(raw.lines().count(), 2, "each append adds exactly one line");
        assert!(raw.ends_with('\n'));
        let records = read_operation_audit(&paths).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].at_ms, 1);
        assert_eq!(records[1].at_ms, 2);
    }

    #[test]
    fn a_crash_torn_tail_row_is_dropped_but_terminated_corruption_fails_closed() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        append_operation_audit(&paths, &input(1), 1).unwrap();
        // 模拟崩溃撕裂：追加一段没有换行结尾的半截 JSON
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(operation_audit_path(&paths))
            .unwrap();
        file.write_all(b"{\"schema_version\":").unwrap();
        let records = read_operation_audit(&paths).unwrap();
        assert_eq!(records.len(), 1, "the torn tail is dropped");
        assert_eq!(records[0].at_ms, 1);
        // 追加仍然可用，新行接在撕裂尾部之后不会污染已有记录
        append_operation_audit(&paths, &input(2), 2).unwrap();
        let records = read_operation_audit(&paths).unwrap();
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn exceeding_the_compaction_threshold_rewrites_with_retention() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let old = input(1);
        append_operation_audit_with_limit(&paths, &old, OPERATION_AUDIT_RETENTION_MS + 2, 0)
            .unwrap();
        append_operation_audit_with_limit(
            &paths,
            &input(OPERATION_AUDIT_RETENTION_MS + 2),
            OPERATION_AUDIT_RETENTION_MS + 2,
            0,
        )
        .unwrap();
        // 阈值 0 强制每次都走压缩路径：过期记录直接从磁盘消失，而不是等 list
        let raw = std::fs::read_to_string(operation_audit_path(&paths)).unwrap();
        assert_eq!(raw.lines().count(), 1);
        let records = read_operation_audit(&paths).unwrap();
        assert_eq!(records[0].at_ms, OPERATION_AUDIT_RETENTION_MS + 2);
    }

    #[test]
    fn reads_keep_only_the_newest_records_past_the_cap() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        for index in 0..MAX_OPERATION_AUDIT_ENTRIES + 2 {
            append_operation_audit(&paths, &input(index as u64), index as u64).unwrap();
        }
        let records = read_operation_audit(&paths).unwrap();
        assert_eq!(records.len(), MAX_OPERATION_AUDIT_ENTRIES);
        assert_eq!(records[0].at_ms, 2, "the two oldest records are dropped");
    }

    #[test]
    fn appending_to_a_corrupt_store_succeeds_but_reads_still_fail_closed() {
        // 追加路径不解析存量：损坏要么来自外部篡改（读取端拒绝），要么来自崩溃撕裂
        // （读取端丢弃尾行）。写入失败才落 sidecar。
        let temp = tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let path = operation_audit_path(&paths);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"{not-json}\n").unwrap();

        append_operation_audit(&paths, &input(10), 10).unwrap();
        assert_eq!(
            read_operation_audit(&paths).unwrap_err(),
            OperationAuditStoreError::InvalidRecord
        );
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
