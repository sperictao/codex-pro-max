#[path = "../src/codex_guard/audit.rs"]
mod audit;
#[path = "../src/codex_guard/audit_runner.rs"]
mod audit_runner;
#[path = "../src/codex_guard/audit_sqlite.rs"]
mod audit_sqlite;

use rusqlite::Connection;
use std::fs;
use tempfile::tempdir;

#[test]
fn runtime_audit_boundary_is_executable() {
    let limits = audit::AuditLimits::default();
    let parsed = audit::parse_rollout_bytes(
        br#"{"type":"session_meta","payload":{"id":"parent-1"}}
{"type":"turn_context","payload":{"turn_id":"turn-1","model":"gpt-test","effort":"high"}}"#,
        limits,
    );
    assert!(!parsed
        .notices
        .iter()
        .any(|notice| notice.code == "parser_error"));
}

#[test]
fn public_runner_merges_rollout_and_both_sqlite_sources_without_leaking_body() {
    let temp = tempdir().expect("isolated codex root");
    let rollout_dir = temp.path().join("rollouts").join("2026-08-11");
    fs::create_dir_all(&rollout_dir).unwrap();
    fs::write(
        rollout_dir.join("rollout.jsonl"),
        include_bytes!("../src/codex_guard/fixtures/audit/rollout-base.jsonl"),
    )
    .unwrap();

    let body = "try_run_sampling_request{turn_id=turn-1 model=gpt-request} model_client.stream_responses_websocket{model=gpt-client} secret_prompt=hidden";
    for path in [
        temp.path().join("logs_2.sqlite"),
        temp.path().join("sqlite").join("logs_2.sqlite"),
    ] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch("CREATE TABLE logs (thread_id TEXT, feedback_tags TEXT, body TEXT);")
            .unwrap();
        connection
            .execute(
                "INSERT INTO logs(thread_id, feedback_tags, body) VALUES (?1, ?2, ?3)",
                ("child-1", "sampling", body),
            )
            .unwrap();
    }

    let result = audit_runner::audit_at(
        temp.path(),
        &[audit::ManagedRoleSnapshot::new(
            "worker", "gpt-test", "high",
        )],
        2_000_000,
    );
    assert_eq!(
        result.source_support,
        audit::AuditSourceSupport::RolloutOnly
    );
    assert_eq!(result.agents.len(), 1);
    let turn = &result.agents[0].turns[0];
    // The sampling request and the WebSocket request carry different models. Both are
    // outbound evidence for the same turn, so the disagreement must surface as ambiguous
    // rather than silently reporting whichever marker was scanned first.
    assert_eq!(turn.outbound_requested_model.as_deref(), None);
    assert_eq!(turn.outbound_evidence_count, 2);
    assert_eq!(turn.outbound_verdict, audit::AuditVerdict::Ambiguous);
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("secret_prompt"));
}

/// When the sampling request and the WebSocket request agree, the two observations
/// dedupe into one model and the turn is judged against it normally.
#[test]
fn public_runner_reports_a_single_outbound_model_when_both_markers_agree() {
    let temp = tempdir().expect("isolated codex root");
    let day = temp.path().join("rollouts").join("2026-08-01");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        day.join("rollout.jsonl"),
        include_bytes!("../src/codex_guard/fixtures/audit/rollout-base.jsonl"),
    )
    .unwrap();

    let body = "try_run_sampling_request{turn_id=turn-1 model=gpt-test} model_client.stream_responses_websocket{model=gpt-test}";
    let path = temp.path().join("logs_2.sqlite");
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch("CREATE TABLE logs (thread_id TEXT, feedback_tags TEXT, body TEXT);")
        .unwrap();
    connection
        .execute(
            "INSERT INTO logs(thread_id, feedback_tags, body) VALUES (?1, ?2, ?3)",
            ("child-1", "sampling", body),
        )
        .unwrap();

    let result = audit_runner::audit_at(
        temp.path(),
        &[audit::ManagedRoleSnapshot::new(
            "worker", "gpt-test", "high",
        )],
        2_000_000,
    );

    let turn = &result.agents[0].turns[0];
    assert_eq!(turn.outbound_requested_model.as_deref(), Some("gpt-test"));
    assert_eq!(turn.outbound_evidence_count, 1);
    assert_eq!(turn.outbound_verdict, audit::AuditVerdict::Match);
}

#[test]
fn public_runner_marks_date_budget_as_truncated() {
    let temp = tempdir().expect("isolated codex root");
    for day in 1..=8 {
        let dir = temp
            .path()
            .join("rollouts")
            .join(format!("2026-08-{day:02}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("rollout.jsonl"), b"{}\n").unwrap();
    }
    let result = audit_runner::audit_at(temp.path(), &[], 1);
    assert!(result
        .notices
        .iter()
        .any(|notice| notice.code == "audit_truncated"));
}
