//! 版本化、脱敏的事务 journal。
//!
//! Journal 只记录相对目标、hash、快照引用和位图，不记录文件内容、绝对路径、命令或
//! 用户配置值。写入先落同目录临时文件并 `sync_all`，再替换 journal，供后续 recovery
//! 读取；完整的跨文件执行协议仍由 transaction/coordinator 接入。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::atomic_store::{AtomicFileWriter, PlatformAtomicFileWriter};
use super::transaction::{TransactionError, TransactionErrorCode, TransactionPhase};

pub(crate) const JOURNAL_SCHEMA_VERSION: u16 = 1;
static NEXT_JOURNAL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalEnvelope {
    pub(crate) schema_version: u16,
    pub(crate) batch_id: String,
    pub(crate) phase: TransactionPhase,
    pub(crate) entries: Vec<JournalEntry>,
    pub(crate) commit_marker: bool,
    pub(crate) critical: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalEntry {
    pub(crate) relative_file: String,
    pub(crate) original_exists: bool,
    pub(crate) original_sha256: String,
    pub(crate) candidate_sha256: String,
    pub(crate) snapshot_ref: String,
    pub(crate) completed: bool,
    pub(crate) post_checked: bool,
    pub(crate) restored: bool,
}

impl JournalEnvelope {
    pub(crate) fn new(batch_id: String, entries: Vec<JournalEntry>) -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            batch_id,
            phase: TransactionPhase::Preflight,
            entries,
            commit_marker: false,
            critical: false,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), TransactionError> {
        if self.schema_version != JOURNAL_SCHEMA_VERSION
            || !valid_component(&self.batch_id)
            || self.commit_marker && self.phase != TransactionPhase::Completed
        {
            return Err(TransactionError::new(TransactionErrorCode::JournalSchema));
        }
        for entry in &self.entries {
            if !valid_relative_ref(&entry.relative_file)
                || !valid_relative_ref(&entry.snapshot_ref)
                || !valid_hash(&entry.original_sha256)
                || !valid_hash(&entry.candidate_sha256)
            {
                return Err(TransactionError::new(TransactionErrorCode::JournalSchema));
            }
        }
        Ok(())
    }

    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>, TransactionError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map_err(|_| TransactionError::new(TransactionErrorCode::JournalSchema))
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, TransactionError> {
        let journal: Self = serde_json::from_slice(bytes)
            .map_err(|_| TransactionError::new(TransactionErrorCode::JournalSchema))?;
        journal.validate()?;
        Ok(journal)
    }
}

pub(crate) fn new_batch_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let serial = NEXT_JOURNAL_ID.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{}", std::process::id(), timestamp, serial)
}

pub(crate) fn write_journal(
    root: &Path,
    journal: &JournalEnvelope,
) -> Result<PathBuf, TransactionError> {
    journal.validate()?;
    fs::create_dir_all(root).map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    restrict_directory(root)?;
    let batch_dir = root.join(&journal.batch_id);
    fs::create_dir_all(&batch_dir)
        .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    restrict_directory(&batch_dir)?;
    let journal_path = batch_dir.join("journal.json");
    let bytes = journal.to_bytes()?;
    PlatformAtomicFileWriter
        .replace(&journal_path, &bytes)
        .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    restrict_file(&journal_path)?;
    sync_directory(&batch_dir)?;
    Ok(journal_path)
}

/// 在 batch 目录中写入不可变快照。快照先写入并同步，再由 journal 引用。
pub(crate) fn write_snapshot(
    root: &Path,
    batch_id: &str,
    snapshot_ref: &str,
    bytes: &[u8],
) -> Result<PathBuf, TransactionError> {
    validate_batch_and_ref(batch_id, snapshot_ref)?;
    let path = root.join(batch_id).join(snapshot_ref);
    let parent = path
        .parent()
        .ok_or_else(|| TransactionError::new(TransactionErrorCode::JournalSchema))?;
    fs::create_dir_all(parent)
        .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    restrict_directory(parent)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    file.write_all(bytes)
        .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    restrict_file(&path)?;
    file.sync_all()
        .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    drop(file);
    sync_directory(parent)?;
    Ok(path)
}

pub(crate) fn read_snapshot(
    root: &Path,
    batch_id: &str,
    snapshot_ref: &str,
) -> Result<Vec<u8>, TransactionError> {
    validate_batch_and_ref(batch_id, snapshot_ref)?;
    fs::read(root.join(batch_id).join(snapshot_ref))
        .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))
}

pub(crate) fn read_journal(path: &Path) -> Result<JournalEnvelope, TransactionError> {
    let bytes =
        fs::read(path).map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    JournalEnvelope::from_bytes(&bytes)
}

pub(crate) fn cleanup_batch(root: &Path, batch_id: &str) -> Result<(), TransactionError> {
    if !valid_component(batch_id) {
        return Err(TransactionError::new(TransactionErrorCode::JournalSchema));
    }
    fs::remove_dir_all(root.join(batch_id))
        .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))
}

fn valid_component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', '\0'])
}

fn valid_relative_ref(value: &str) -> bool {
    if value.is_empty() || value.contains('\0') {
        return false;
    }
    let path = Path::new(value);
    !path.is_absolute()
        && !value.contains(':')
        && !value
            .split(['/', '\\'])
            .any(|part| part.is_empty() || part == "." || part == "..")
}

fn validate_batch_and_ref(batch_id: &str, snapshot_ref: &str) -> Result<(), TransactionError> {
    if !valid_component(batch_id) || !valid_relative_ref(snapshot_ref) {
        return Err(TransactionError::new(TransactionErrorCode::JournalSchema));
    }
    Ok(())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sync_directory(path: &Path) -> Result<(), TransactionError> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn restrict_file(path: &Path) -> Result<(), TransactionError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn restrict_directory(path: &Path) -> Result<(), TransactionError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| TransactionError::new(TransactionErrorCode::JournalIo))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::transaction::TransactionPhase;

    fn entry() -> JournalEntry {
        JournalEntry {
            relative_file: "config.toml".into(),
            original_exists: true,
            original_sha256: "0".repeat(64),
            candidate_sha256: "1".repeat(64),
            snapshot_ref: "snapshots/config.toml.bin".into(),
            completed: false,
            post_checked: false,
            restored: false,
        }
    }

    #[test]
    fn journal_round_trip_is_versioned_and_does_not_contain_absolute_or_secret_data() {
        let journal = JournalEnvelope::new("batch-1".into(), vec![entry()]);
        let bytes = journal.to_bytes().unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("\"schema_version\":1"));
        assert!(!text.contains("/Users/"));
        assert!(!text.contains("TOP_SECRET"));
        assert_eq!(
            JournalEnvelope::from_bytes(&bytes).unwrap().batch_id,
            "batch-1"
        );
    }

    #[test]
    fn unknown_schema_and_absolute_refs_fail_closed() {
        let mut journal = JournalEnvelope::new("batch-1".into(), vec![entry()]);
        journal.schema_version = 99;
        assert_eq!(
            journal.validate().unwrap_err().code,
            TransactionErrorCode::JournalSchema
        );

        let mut journal = JournalEnvelope::new("batch-1".into(), vec![entry()]);
        journal.entries[0].relative_file = "/outside/config.toml".into();
        assert_eq!(
            journal.validate().unwrap_err().code,
            TransactionErrorCode::JournalSchema
        );
    }

    #[test]
    fn journal_write_reads_and_cleans_a_batch_directory() {
        let temp = tempfile::tempdir().unwrap();
        let journal = JournalEnvelope::new("batch-1".into(), vec![entry()]);
        let path = write_journal(temp.path(), &journal).unwrap();
        assert_eq!(
            read_journal(&path).unwrap().phase,
            TransactionPhase::Preflight
        );
        cleanup_batch(temp.path(), "batch-1").unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn snapshot_round_trip_rejects_escape_references() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = b"original bytes";
        write_snapshot(temp.path(), "batch-1", "snapshots/entry-0.bin", bytes).unwrap();
        assert_eq!(
            read_snapshot(temp.path(), "batch-1", "snapshots/entry-0.bin").unwrap(),
            bytes
        );
        assert_eq!(
            write_snapshot(temp.path(), "batch-1", "../outside", bytes)
                .unwrap_err()
                .code,
            TransactionErrorCode::JournalSchema
        );
    }
}
