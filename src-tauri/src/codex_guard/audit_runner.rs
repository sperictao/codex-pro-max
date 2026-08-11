//! Production boundary for the bounded runtime audit.
//!
//! The parser in `audit.rs` stays path agnostic.  This module is the only place that discovers
//! the current Codex rollout/SQLite locations and turns them into bounded parser inputs.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::audit::{
    merge_outbound_evidence, merge_rollout_evidence, parse_rollout_jsonl, AuditLimits, AuditNotice,
    AuditSourceSupport, AuditVerdict, ManagedRoleSnapshot, OutboundObservation, ParsedRollout,
    SubagentAuditResult,
};
use super::audit_sqlite::{read_sampling_sources, SqliteNoticeCode};

const ROLLOUT_DIRECTORY: &str = "rollouts";
#[cfg(test)]
const MAX_READ_BYTES: usize = super::audit::MAX_ROLLOUT_FILE_BYTES;

static AUDIT_RUNNING: AtomicBool = AtomicBool::new(false);

struct AuditFlight;

impl Drop for AuditFlight {
    fn drop(&mut self) {
        AUDIT_RUNNING.store(false, Ordering::Release);
    }
}

pub(crate) fn run_subagent_audit(
    codex_root: &Path,
    roles: &[ManagedRoleSnapshot],
    checked_at_ms: u64,
) -> Result<SubagentAuditResult, String> {
    if AUDIT_RUNNING.swap(true, Ordering::AcqRel) {
        return Err("audit_already_running".to_string());
    }
    let _flight = AuditFlight;
    Ok(audit_at(codex_root, roles, checked_at_ms))
}

pub(crate) fn audit_at(
    codex_root: &Path,
    roles: &[ManagedRoleSnapshot],
    checked_at_ms: u64,
) -> SubagentAuditResult {
    let limits = AuditLimits::default();
    let rollout = read_rollout_files(&codex_root.join(ROLLOUT_DIRECTORY), limits);
    let mut parsed = rollout.parsed;
    for code in rollout.notices {
        push_notice(&mut parsed.notices, code);
    }
    if rollout.truncated {
        parsed.truncated = true;
        push_notice(&mut parsed.notices, "audit_truncated");
    }
    let mut result = merge_rollout_evidence(&parsed, roles, checked_at_ms);

    let known_thread_ids = parsed
        .observations
        .iter()
        .filter_map(|observation| match observation {
            super::audit::RolloutObservation::Session(session)
                if session.parent_thread_id.is_some() =>
            {
                Some(session.thread_id.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if known_thread_ids.is_empty() {
        return result;
    }

    let sources = [
        ("logs_2", codex_root.join("logs_2.sqlite")),
        (
            "sqlite/logs_2",
            codex_root.join("sqlite").join("logs_2.sqlite"),
        ),
    ];
    let source_refs = sources
        .iter()
        .map(|(name, path)| (*name, path.as_path()))
        .collect::<Vec<_>>();
    let sqlite = read_sampling_sources(&source_refs, &known_thread_ids);
    for notice in &sqlite.notices {
        let code = match notice.code {
            SqliteNoticeCode::SourceMissing => "audit_sqlite_source_missing",
            SqliteNoticeCode::SourceBusy => "audit_sqlite_source_busy",
            SqliteNoticeCode::MissingLogsTable => "audit_sqlite_missing_logs_table",
            SqliteNoticeCode::SchemaDrift => "audit_sqlite_schema_drift",
            SqliteNoticeCode::BodyTooLarge => "audit_sqlite_body_too_large",
            SqliteNoticeCode::InvalidMarker => "audit_sqlite_invalid_marker",
            SqliteNoticeCode::ConflictingEvidence => "audit_sqlite_conflicting_evidence",
        };
        push_notice(&mut result.notices, code);
    }
    // 缺一个库不得覆盖另一个库的有效证据：真实安装通常只有两个路径中的一个，
    // 直接早退会让 outbound 证据链永远不可用。仍合并已读到的证据，
    // 但保留 incomplete notice，并禁止把结果升级为 Full。
    let complete = sqlite.complete;
    if !complete {
        push_notice(&mut result.notices, "audit_sqlite_incomplete");
    }

    // 采样请求模型与实际发往 /responses 的 WebSocket 模型是两类独立证据。
    // 两者都作为同一 turn 的观测传入：一致时去重成一个模型，不一致时
    // merge_outbound_evidence 会因该 turn 出现多个模型而判为 Ambiguous，冲突因此可见。
    let outbound = sqlite
        .evidence
        .iter()
        .flat_map(|evidence| {
            [&evidence.requested_model, &evidence.client_model]
                .into_iter()
                .filter(|model| !model.is_empty())
                .map(|model| OutboundObservation {
                    thread_id: evidence.thread_id.clone(),
                    turn_id: evidence.turn_id.clone(),
                    requested_model: model.clone(),
                    observed_at_ms: None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut merged = merge_outbound_evidence(&result, &outbound);
    if !complete {
        for agent in &mut merged.agents {
            agent.verdict = agent.verdict.worst(AuditVerdict::Incomplete);
        }
        if matches!(merged.source_support, AuditSourceSupport::Full) {
            merged.source_support = AuditSourceSupport::RolloutOnly;
        }
    }
    merged
}

fn push_notice(notices: &mut Vec<AuditNotice>, code: &'static str) {
    if !notices.iter().any(|notice| notice.code == code) {
        notices.push(AuditNotice::new(code));
    }
}

struct RolloutReadResult {
    parsed: ParsedRollout,
    notices: Vec<&'static str>,
    truncated: bool,
}

fn read_rollout_files(root: &Path, limits: AuditLimits) -> RolloutReadResult {
    let Ok(entries) = fs::read_dir(root) else {
        return RolloutReadResult {
            parsed: ParsedRollout {
                observations: Vec::new(),
                notices: Vec::new(),
                truncated: false,
            },
            notices: Vec::new(),
            truncated: false,
        };
    };
    let mut dates = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .map(|_| {
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        entry.path(),
                    )
                })
        })
        .collect::<Vec<_>>();
    dates.sort_by(|left, right| right.0.cmp(&left.0));
    let date_limit = limits.max_date_directories;
    let date_count = dates.len();
    let mut truncated =
        (date_limit == 0 && date_count > 0) || (date_limit > 0 && date_count >= date_limit);
    dates.truncate(date_limit);

    let mut parsed = ParsedRollout {
        observations: Vec::new(),
        notices: Vec::new(),
        truncated: false,
    };
    let mut notices = Vec::new();
    let mut selected_files = 0usize;
    for (_date_key, date_path) in dates {
        let Ok(entries) = fs::read_dir(date_path) else {
            continue;
        };
        let mut candidates = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let kind = entry.file_type().ok()?;
                let name = entry.file_name().to_string_lossy().into_owned();
                (kind.is_file() && name.ends_with(".jsonl")).then_some((name, entry.path()))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.0.cmp(&left.0));
        for (_file_key, path) in candidates {
            if limits.max_rollout_files == 0 || selected_files >= limits.max_rollout_files {
                truncated = true;
                push_read_notice(&mut notices, "audit_truncated");
                break;
            }
            match read_rollout_stream(&path, limits) {
                Ok((item, file_truncated)) => {
                    merge_parsed(&mut parsed, item);
                    if file_truncated {
                        truncated = true;
                        push_read_notice(&mut notices, "audit_truncated");
                    }
                    selected_files += 1;
                }
                Err(ReadBoundedError::Read) => push_read_notice(&mut notices, "audit_read_error"),
            }
        }
    }
    if limits.max_rollout_files > 0 && selected_files >= limits.max_rollout_files {
        truncated = true;
        push_read_notice(&mut notices, "audit_truncated");
    }
    RolloutReadResult {
        parsed,
        notices,
        truncated,
    }
}

fn push_read_notice(notices: &mut Vec<&'static str>, code: &'static str) {
    if !notices.contains(&code) {
        notices.push(code);
    }
}

enum ReadBoundedError {
    Read,
}

fn read_rollout_stream(
    path: &Path,
    limits: AuditLimits,
) -> Result<(ParsedRollout, bool), ReadBoundedError> {
    let file = File::open(path).map_err(|_| ReadBoundedError::Read)?;
    let max_bytes = limits.max_file_bytes;
    let mut limited = file.take((max_bytes as u64).saturating_add(1));
    let parsed = parse_rollout_jsonl(&mut limited, limits);
    let truncated = limited.limit() == 0;
    Ok((parsed, truncated))
}

fn merge_parsed(destination: &mut ParsedRollout, source: ParsedRollout) {
    destination.observations.extend(source.observations);
    destination.truncated |= source.truncated;
    for notice in source.notices {
        if !destination
            .notices
            .iter()
            .any(|existing| existing.code == notice.code)
        {
            destination.notices.push(notice);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn audit_at_reads_only_bounded_rollout_inputs() {
        let temp = tempdir().unwrap();
        let date = temp.path().join("rollouts").join("2026-08-11");
        fs::create_dir_all(&date).unwrap();
        fs::write(
            date.join("one.jsonl"),
            br#"{"type":"session_meta","payload":{"id":"root"}}
{"type":"session_meta","payload":{"id":"child","parent_thread_id":"root","agent_role":"default","started_at_ms":10}}
{"type":"response_item","payload":{"type":"function_call","name":"spawn_agent","arguments":"{\"task_name\":\"child\",\"agent_type\":\"default\",\"fork_turns\":\"none\"}"}}
{"type":"turn_context","payload":{"thread_id":"child","turn_id":"turn-1","model":"gpt-test","effort":"high","multi_agent_version":"v2","started_at_ms":20}}
"#,
        )
        .unwrap();
        let result = audit_at(
            temp.path(),
            &[ManagedRoleSnapshot::new("default", "gpt-test", "high")],
            30,
        );
        assert_eq!(
            result.schema_version,
            super::super::audit::AUDIT_SCHEMA_VERSION
        );
        assert_eq!(result.agents.len(), 1);
        assert!(result
            .notices
            .iter()
            .any(|notice| notice.code == "audit_sqlite_incomplete"));
    }

    #[test]
    fn single_flight_rejects_a_second_run() {
        assert!(!AUDIT_RUNNING.swap(true, Ordering::AcqRel));
        let result = run_subagent_audit(tempdir().unwrap().path(), &[], 1);
        assert_eq!(result.unwrap_err(), "audit_already_running");
        AUDIT_RUNNING.store(false, Ordering::Release);
    }

    #[test]
    fn oversized_rollout_is_reported_as_truncated() {
        let temp = tempdir().unwrap();
        let date = temp.path().join("rollouts").join("2026-08-11");
        fs::create_dir_all(&date).unwrap();
        let path = date.join("oversized.jsonl");
        let file = File::create(&path).unwrap();
        file.set_len((MAX_READ_BYTES as u64).saturating_add(1))
            .unwrap();

        let result = audit_at(temp.path(), &[], 30);
        assert!(result
            .notices
            .iter()
            .any(|notice| notice.code == "audit_truncated"));
    }
}
