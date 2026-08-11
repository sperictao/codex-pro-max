#[path = "../src/codex_guard/audit.rs"]
mod audit;

#[test]
fn operation_audit_boundary_is_executable_and_redacts_paths() {
    let input = audit::OperationAuditInput {
        at_ms: 1,
        batch_id: "batch-1".to_string(),
        scope: "global".to_string(),
        relative_file: Some("/private/config.toml".to_string()),
        phase: audit::OperationAuditPhase::Completed,
        result: audit::OperationAuditResult::Success,
        error_code: Some("ok".to_string()),
        changed: 1,
        unchanged: 0,
        files: 1,
        role_id: Some("default".to_string()),
        model: Some("gpt-test".to_string()),
        effort: Some("high".to_string()),
    };
    let record = audit::sanitize_operation_audit(&input).expect("sanitized audit record");
    assert_eq!(record.relative_file, None);
}
