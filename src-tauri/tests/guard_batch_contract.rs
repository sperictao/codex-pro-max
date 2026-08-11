//! Behavioural contract for the batch IPC boundary.
//!
//! These tests deliberately go through serde and the public action matrix instead of
//! grepping the source: a rename that is applied consistently across the crate is fine,
//! but a change to the JSON wire shape or to the eligibility rules is a frontend-visible
//! break and must fail here.

#[allow(dead_code)]
#[path = "../src/codex_guard/batch.rs"]
mod batch;
#[allow(dead_code)]
#[path = "../src/codex_guard/lifecycle.rs"]
mod lifecycle;
#[allow(dead_code)]
#[path = "../src/codex_guard/model.rs"]
mod model;

use batch::{
    plan_member_action, scope_matches, BatchAction, BatchMember, BatchOutcome, BatchRequest,
    BatchScope, GuardOperationPhase, BATCH_CONTRACT_SCHEMA_VERSION,
};
use lifecycle::{HealthStatus, ParameterLifecycle};

fn request_json(scope: serde_json::Value, action: &str) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": BATCH_CONTRACT_SCHEMA_VERSION,
        "scope": scope,
        "action": action,
    })
}

/// The frontend sends these exact payloads. Deserializing them is the contract.
#[test]
fn every_scope_variant_round_trips_through_the_documented_wire_shape() {
    let cases = [
        (serde_json::json!("all"), BatchScope::All),
        (
            serde_json::json!({"group": {"groupId": "general"}}),
            BatchScope::Group {
                group_id: "general".into(),
            },
        ),
        (
            serde_json::json!({"parameter": {"parameterId": "model"}}),
            BatchScope::Parameter {
                parameter_id: "model".into(),
            },
        ),
        (
            serde_json::json!({"role": {"roleId": "reviewer"}}),
            BatchScope::Role {
                role_id: "reviewer".into(),
            },
        ),
    ];

    for (wire, expected) in cases {
        let request: BatchRequest =
            serde_json::from_value(request_json(wire.clone(), "apply")).expect("scope must parse");
        assert_eq!(request.scope, expected, "wire shape changed for {wire}");
        assert!(request.is_supported_version());

        // Re-serializing must reproduce the same wire shape the frontend sent.
        assert_eq!(
            serde_json::to_value(&request.scope).unwrap(),
            wire,
            "scope does not round-trip"
        );
    }
}

#[test]
fn every_action_round_trips_through_its_snake_case_name() {
    for (wire, expected) in [
        ("apply", BatchAction::Apply),
        ("lock", BatchAction::Lock),
        ("unlock", BatchAction::Unlock),
        ("disable", BatchAction::Disable),
    ] {
        let request: BatchRequest =
            serde_json::from_value(request_json(serde_json::json!("all"), wire))
                .expect("action must parse");
        assert_eq!(request.action, expected);
        assert_eq!(serde_json::to_value(request.action).unwrap(), wire);
    }
}

#[test]
fn every_outcome_and_progress_phase_keeps_its_wire_name() {
    for (outcome, wire) in [
        (BatchOutcome::Committed, "committed"),
        (BatchOutcome::Rejected, "rejected"),
        (BatchOutcome::RolledBack, "rolled_back"),
        (BatchOutcome::CriticalRecovery, "critical_recovery"),
    ] {
        assert_eq!(serde_json::to_value(outcome).unwrap(), wire);
    }
    for (phase, wire) in [
        (GuardOperationPhase::Preflight, "preflight"),
        (GuardOperationPhase::Snapshot, "snapshot"),
        (GuardOperationPhase::Write, "write"),
        (GuardOperationPhase::Verify, "verify"),
        (GuardOperationPhase::Completed, "completed"),
        (GuardOperationPhase::Recovery, "recovery"),
    ] {
        assert_eq!(serde_json::to_value(phase).unwrap(), wire);
    }
}

/// An unknown schema version must be rejected rather than silently defaulted, and
/// unknown fields must not be accepted from the IPC boundary.
#[test]
fn the_request_boundary_is_fail_closed() {
    let stale: BatchRequest = serde_json::from_value(serde_json::json!({
        "schemaVersion": BATCH_CONTRACT_SCHEMA_VERSION + 1,
        "scope": "all",
        "action": "apply",
    }))
    .expect("a versioned request still parses");
    assert!(
        !stale.is_supported_version(),
        "a future schema version must not be reported as supported"
    );

    assert!(
        serde_json::from_value::<BatchRequest>(serde_json::json!({
            "schemaVersion": BATCH_CONTRACT_SCHEMA_VERSION,
            "scope": "all",
            "action": "apply",
            "unexpected": true,
        }))
        .is_err(),
        "unknown fields must be rejected"
    );

    assert!(
        serde_json::from_value::<BatchRequest>(serde_json::json!({
            "scope": "all",
            "action": "apply",
        }))
        .is_err(),
        "schemaVersion must be explicit"
    );
}

/// Scope resolution decides which members a bulk action touches. Getting this wrong
/// silently applies a lifecycle change to the wrong parameters.
#[test]
fn scope_resolution_selects_exactly_the_intended_members() {
    let member = BatchMember::new("model", ParameterLifecycle::Applied, HealthStatus::Healthy)
        .in_group("general")
        .for_role("reviewer");
    let other = BatchMember::new(
        "sandbox",
        ParameterLifecycle::Applied,
        HealthStatus::Healthy,
    )
    .in_group("subagent-optimization");

    assert!(scope_matches(&BatchScope::All, &member));
    assert!(scope_matches(&BatchScope::All, &other));

    let group = BatchScope::Group {
        group_id: "general".into(),
    };
    assert!(scope_matches(&group, &member));
    assert!(!scope_matches(&group, &other));

    let parameter = BatchScope::Parameter {
        parameter_id: "model".into(),
    };
    assert!(scope_matches(&parameter, &member));
    assert!(!scope_matches(&parameter, &other));

    let role = BatchScope::Role {
        role_id: "reviewer".into(),
    };
    assert!(scope_matches(&role, &member));
    assert!(!scope_matches(&role, &other));
}

/// The action matrix decides both the resulting lifecycle and whether the Codex file
/// is written. A regression here either loses guard coverage or writes a file the user
/// never enabled.
#[test]
fn the_action_matrix_maps_each_lifecycle_to_its_documented_target() {
    use BatchAction::{Apply, Disable, Lock, Unlock};
    use ParameterLifecycle::{Applied, Disabled, Locked};

    let cases = [
        (Apply, Disabled, Applied, true),
        (Apply, Applied, Applied, false),
        (Apply, Locked, Locked, false),
        (Lock, Disabled, Locked, true),
        (Lock, Applied, Locked, false),
        (Lock, Locked, Locked, false),
        (Unlock, Locked, Applied, false),
        (Unlock, Applied, Applied, false),
        (Unlock, Disabled, Disabled, false),
        (Disable, Locked, Disabled, false),
        (Disable, Applied, Disabled, false),
        (Disable, Disabled, Disabled, false),
    ];

    for (action, from, expected, writes_file) in cases {
        let plan = plan_member_action(action, from, HealthStatus::Healthy);
        assert_eq!(
            plan.lifecycle, expected,
            "{action:?} from {from:?} must land on {expected:?}"
        );
        assert_eq!(
            plan.writes_file, writes_file,
            "{action:?} from {from:?} has the wrong file-write effect"
        );
    }

    // A drifted, already-applied member must be rewritten; a healthy one must not.
    assert!(plan_member_action(Apply, Applied, HealthStatus::Drifted).writes_file);
    assert!(!plan_member_action(Apply, Applied, HealthStatus::Healthy).writes_file);
}
