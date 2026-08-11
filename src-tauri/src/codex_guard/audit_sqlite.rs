//! Read-only sampling evidence from the two Codex SQLite locations.
//!
//! This module never copies or mutates a database.  It opens each source with SQLite
//! read-only/query-only flags, uses parameterized thread IDs, bounds each body, and
//! scans only the two known sampling markers required by the audit contract.

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

pub const MAX_BODY_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SqliteNoticeCode {
    SourceMissing,
    SourceBusy,
    MissingLogsTable,
    SchemaDrift,
    BodyTooLarge,
    InvalidMarker,
    ConflictingEvidence,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SqliteNotice {
    pub source: String,
    pub code: SqliteNoticeCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SamplingEvidence {
    pub thread_id: String,
    pub turn_id: String,
    pub requested_model: String,
    pub client_model: String,
    pub source_count: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SqliteAuditResult {
    pub evidence: Vec<SamplingEvidence>,
    pub notices: Vec<SqliteNotice>,
    pub complete: bool,
}

pub fn read_sampling_sources(
    sources: &[(&str, &Path)],
    known_turn_ids: &BTreeSet<String>,
) -> SqliteAuditResult {
    let mut notices = Vec::new();
    let mut evidence_by_key: BTreeMap<(String, String, String), SamplingEvidence> = BTreeMap::new();
    let mut source_ok = 0usize;
    for (source_name, path) in sources {
        match read_one_source(source_name, path, known_turn_ids) {
            Ok(rows) => {
                source_ok += 1;
                for evidence in rows {
                    let key = (
                        evidence.thread_id.clone(),
                        evidence.turn_id.clone(),
                        evidence.requested_model.clone(),
                    );
                    if let Some(existing) = evidence_by_key.get_mut(&key) {
                        if existing.client_model != evidence.client_model {
                            notices.push(SqliteNotice {
                                source: (*source_name).to_string(),
                                code: SqliteNoticeCode::ConflictingEvidence,
                            });
                            existing.client_model.clear();
                        } else {
                            existing.source_count = existing.source_count.saturating_add(1);
                        }
                    } else {
                        evidence_by_key.insert(key, evidence);
                    }
                }
            }
            Err(code) => notices.push(SqliteNotice {
                source: (*source_name).to_string(),
                code,
            }),
        }
    }
    let mut evidence = evidence_by_key.into_values().collect::<Vec<_>>();
    evidence.sort_by(|left, right| {
        (&left.thread_id, &left.turn_id, &left.requested_model).cmp(&(
            &right.thread_id,
            &right.turn_id,
            &right.requested_model,
        ))
    });
    let complete = source_ok == sources.len()
        && notices.is_empty()
        && known_turn_ids
            .iter()
            .all(|thread_id| evidence.iter().any(|item| &item.thread_id == thread_id));
    SqliteAuditResult {
        evidence,
        notices,
        complete,
    }
}

fn read_one_source(
    _source_name: &str,
    path: &Path,
    known_turn_ids: &BTreeSet<String>,
) -> Result<Vec<SamplingEvidence>, SqliteNoticeCode> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        if error.to_string().to_ascii_lowercase().contains("busy") {
            SqliteNoticeCode::SourceBusy
        } else {
            SqliteNoticeCode::SourceMissing
        }
    })?;
    conn.busy_timeout(Duration::from_millis(100))
        .map_err(|_| SqliteNoticeCode::SourceBusy)?;
    conn.execute_batch("PRAGMA query_only=ON;")
        .map_err(|_| SqliteNoticeCode::SchemaDrift)?;
    let columns = table_columns(&conn)?;
    if !columns.contains("thread_id")
        || !columns.contains("feedback_tags")
        || !columns.contains("body")
    {
        return Err(SqliteNoticeCode::SchemaDrift);
    }
    let mut stmt = conn
        .prepare("SELECT feedback_tags, body FROM logs WHERE thread_id = ?1")
        .map_err(|_| SqliteNoticeCode::SchemaDrift)?;
    let mut output = Vec::new();
    for thread_id in known_turn_ids {
        let rows = stmt
            .query_map([thread_id], |row| {
                let tags: Option<String> = row.get(0)?;
                let body: Option<String> = row.get(1)?;
                Ok((tags.unwrap_or_default(), body.unwrap_or_default()))
            })
            .map_err(|_| SqliteNoticeCode::SchemaDrift)?;
        for row in rows {
            let (tags, body) = row.map_err(|_| SqliteNoticeCode::SchemaDrift)?;
            if body.len() > MAX_BODY_BYTES {
                return Err(SqliteNoticeCode::BodyTooLarge);
            }
            if !tags.contains("sampling") && !tags.contains("feedback") {
                continue;
            }
            // turn_id 与 model 必须来自同一个 marker 实例，否则无法证明它们描述同一次采样。
            let request =
                parse_marker_fields(&body, "try_run_sampling_request", &["turn_id", "model"])?;
            let client_model =
                parse_marker_fields(&body, "model_client.stream_responses_websocket", &["model"])?;
            let Some(request) = request else {
                return Err(SqliteNoticeCode::InvalidMarker);
            };
            let turn_id = request[0].clone();
            let requested_model = request[1].clone();
            if turn_id.is_empty() {
                return Err(SqliteNoticeCode::InvalidMarker);
            }
            let client_model = client_model
                .map(|values| values[0].clone())
                .ok_or(SqliteNoticeCode::InvalidMarker)?;
            output.push(SamplingEvidence {
                thread_id: thread_id.clone(),
                turn_id,
                requested_model,
                client_model,
                source_count: 1,
            });
        }
    }
    Ok(output)
}

fn table_columns(conn: &Connection) -> Result<BTreeSet<String>, SqliteNoticeCode> {
    let mut statement = conn
        .prepare("PRAGMA table_info(logs)")
        .map_err(|_| SqliteNoticeCode::MissingLogsTable)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|_| SqliteNoticeCode::MissingLogsTable)?;
    let mut columns = BTreeSet::new();
    for row in rows {
        columns.insert(row.map_err(|_| SqliteNoticeCode::MissingLogsTable)?);
    }
    if columns.is_empty() {
        return Err(SqliteNoticeCode::MissingLogsTable);
    }
    Ok(columns)
}

/// 从同一个 marker 实例中一次取出全部所需字段。分多次独立扫描再把返回值拼在一起，
/// 会把 marker#1 的 turn_id 和 marker#2 的 model 拼成一条并不存在的"证据"。
/// 任一 marker 实例缺字段、或多个实例给出不同值，都必须是 ambiguous。
fn parse_marker_fields(
    body: &str,
    marker: &str,
    keys: &[&str],
) -> Result<Option<Vec<String>>, SqliteNoticeCode> {
    let prefix = format!("{marker}{{");
    let mut offset = 0usize;
    let mut found: Option<Vec<String>> = None;
    while let Some(relative) = body[offset..].find(&prefix) {
        let start = offset + relative;
        let rest = &body[start + prefix.len()..];
        let Some(end) = rest.find('}') else {
            return Err(SqliteNoticeCode::InvalidMarker);
        };
        let fields = &rest[..end];
        let mut values = vec![None; keys.len()];
        for field in fields.split_whitespace() {
            let Some((name, value)) = field.split_once('=') else {
                return Err(SqliteNoticeCode::InvalidMarker);
            };
            let Some(index) = keys.iter().position(|key| *key == name) else {
                continue;
            };
            if value.is_empty() || value.len() > 128 || !value.is_ascii() {
                return Err(SqliteNoticeCode::InvalidMarker);
            }
            if values[index]
                .as_deref()
                .is_some_and(|previous: &str| previous != value)
            {
                return Err(SqliteNoticeCode::ConflictingEvidence);
            }
            values[index] = Some(value.to_string());
        }
        // 一个 marker 实例要么给全所需字段，要么一个都不给；给一半说明无法一一对应。
        let present = values.iter().filter(|value| value.is_some()).count();
        if present != 0 && present != keys.len() {
            return Err(SqliteNoticeCode::InvalidMarker);
        }
        if present == keys.len() {
            let complete = values
                .into_iter()
                .map(|value| value.expect("checked complete"))
                .collect::<Vec<_>>();
            if found.as_ref().is_some_and(|previous| *previous != complete) {
                return Err(SqliteNoticeCode::ConflictingEvidence);
            }
            found = Some(complete);
        }
        offset = start + prefix.len() + end + 1;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn db(path: &Path, body: &str) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch("CREATE TABLE logs (thread_id TEXT, feedback_tags TEXT, body TEXT);")
            .unwrap();
        connection
            .execute(
                "INSERT INTO logs(thread_id, feedback_tags, body) VALUES (?1, ?2, ?3)",
                ("turn-1", "sampling", body),
            )
            .unwrap();
    }

    #[test]
    fn reads_sampling_markers_read_only_and_reports_match() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("logs.sqlite");
        db(
            &path,
            "try_run_sampling_request{turn_id=turn-1 model=gpt-request} model_client.stream_responses_websocket{model=gpt-client}",
        );
        let before = std::fs::read(&path).unwrap();
        let mut ids = BTreeSet::new();
        ids.insert("turn-1".to_string());
        let result = read_sampling_sources(&[("primary", &path)], &ids);
        assert!(result.complete);
        assert_eq!(result.evidence[0].client_model, "gpt-client");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn binds_thread_id_but_returns_turn_id_from_the_marker() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("logs.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE logs (thread_id TEXT, feedback_tags TEXT, body TEXT);")
            .unwrap();
        connection
            .execute(
                "INSERT INTO logs(thread_id, feedback_tags, body) VALUES (?1, ?2, ?3)",
                (
                    "thread-1",
                    "sampling",
                    "try_run_sampling_request{turn_id=turn-9 model=gpt-request} model_client.stream_responses_websocket{model=gpt-client}",
                ),
            )
            .unwrap();
        let ids = BTreeSet::from(["thread-1".to_string()]);
        let result = read_sampling_sources(&[("primary", &path)], &ids);
        assert!(result.complete);
        assert_eq!(result.evidence[0].thread_id, "thread-1");
        assert_eq!(result.evidence[0].turn_id, "turn-9");
    }

    #[test]
    fn conflicting_markers_never_look_complete() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("logs.sqlite");
        db(
            &path,
            "try_run_sampling_request{turn_id=turn-1 model=gpt-a} try_run_sampling_request{turn_id=turn-1 model=gpt-b} model_client.stream_responses_websocket{model=gpt-client}",
        );
        let ids = BTreeSet::from(["turn-1".to_string()]);
        let result = read_sampling_sources(&[("primary", &path)], &ids);
        assert!(!result.complete);
        assert!(result
            .notices
            .iter()
            .any(|notice| { notice.code == SqliteNoticeCode::ConflictingEvidence }));
    }

    #[test]
    fn missing_schema_and_body_limit_never_look_green() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("logs.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE other (value TEXT);")
            .unwrap();
        let ids = BTreeSet::new();
        let result = read_sampling_sources(&[("primary", &path)], &ids);
        assert!(!result.complete);
        assert_eq!(result.notices[0].code, SqliteNoticeCode::MissingLogsTable);
    }

    /// `turn_id` and `model` must come from the same marker instance. Scanning for each
    /// key independently and pairing the results lets a marker that only carries a
    /// `turn_id` be joined with a different marker's `model`, fabricating evidence that
    /// no single sampling request ever produced.
    #[test]
    fn fields_are_never_stitched_across_two_marker_instances() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("logs.sqlite");
        db(
            &path,
            "try_run_sampling_request{turn_id=turn-1} try_run_sampling_request{model=gpt-from-another-marker} model_client.stream_responses_websocket{model=gpt-client}",
        );
        let mut ids = BTreeSet::new();
        ids.insert("turn-1".to_string());

        let result = read_sampling_sources(&[("primary", &path)], &ids);

        assert!(
            !result.complete,
            "a half-populated marker must not produce complete evidence"
        );
        assert!(
            result.evidence.is_empty(),
            "fields from separate marker instances must not be stitched together"
        );
    }

    #[test]
    fn conflicting_duplicate_sources_are_visible() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first.sqlite");
        let second = temp.path().join("second.sqlite");
        db(&first, "try_run_sampling_request{turn_id=turn-1 model=gpt-request} model_client.stream_responses_websocket{model=gpt-a}");
        db(&second, "try_run_sampling_request{turn_id=turn-1 model=gpt-request} model_client.stream_responses_websocket{model=gpt-b}");
        let mut ids = BTreeSet::new();
        ids.insert("turn-1".to_string());
        let result = read_sampling_sources(&[("primary", &first), ("secondary", &second)], &ids);
        assert!(!result.complete);
        assert!(result
            .notices
            .iter()
            .any(|notice| notice.code == SqliteNoticeCode::ConflictingEvidence));
    }
}
