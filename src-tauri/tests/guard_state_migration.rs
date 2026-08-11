//! Behavioural contract for the persisted Guard state migration.
//!
//! The v0 → v1 migration decides, for every legacy `{applied, locked}` pair, whether the
//! parameter keeps guarding a file, stops guarding it, or is held back for an explicit user
//! choice. Getting that mapping wrong silently changes what the app enforces on ~/.codex.
//!
//! The full on-disk migration (durable backup + journalled replace) is covered inside the
//! main crate, where `ConfigStore` can be driven directly. What this target adds is the
//! shipped v0 fixture: every legacy combination is pushed through the real lifecycle
//! decision function and asserted, so a change to that mapping fails here even when it is
//! applied consistently across the crate.

#[allow(dead_code)]
#[path = "../src/codex_guard/lifecycle.rs"]
mod lifecycle;

use lifecycle::ParameterLifecycle;

const V0_FIXTURE: &[u8] = include_bytes!("../src/codex_guard/fixtures/migration/launcher-v0.json");

fn v0_params() -> serde_json::Map<String, serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(V0_FIXTURE).expect("v0 fixture JSON");
    assert!(
        value["codex_guard"].get("schema_version").is_none(),
        "the fixture must stay a genuine v0 document"
    );
    value["codex_guard"]["params"]
        .as_object()
        .expect("params object")
        .clone()
}

fn flags(param: &serde_json::Value) -> (bool, bool) {
    (
        param["applied"].as_bool().expect("applied flag"),
        param["locked"].as_bool().expect("locked flag"),
    )
}

#[test]
fn the_v0_fixture_still_carries_every_legacy_boolean_combination() {
    let params = v0_params();
    let combinations = params
        .values()
        .map(flags)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        combinations.len(),
        4,
        "the fixture must exercise all four legacy boolean combinations"
    );
}

/// The three legal pairs map onto the three lifecycles; the illegal pair maps onto none.
/// This is the whole migration decision, asserted on its real inputs.
#[test]
fn legacy_flag_pairs_map_onto_exactly_three_lifecycles() {
    assert_eq!(
        ParameterLifecycle::from_legacy_flags(false, false),
        Ok(ParameterLifecycle::Disabled)
    );
    assert_eq!(
        ParameterLifecycle::from_legacy_flags(true, false),
        Ok(ParameterLifecycle::Applied)
    );
    assert_eq!(
        ParameterLifecycle::from_legacy_flags(true, true),
        Ok(ParameterLifecycle::Locked)
    );
    assert!(
        ParameterLifecycle::from_legacy_flags(false, true).is_err(),
        "locked-without-applied must stay an explicit migration blocker"
    );
}

/// Every parameter in the shipped fixture must migrate to the lifecycle its name claims,
/// and the invalid pair must refuse to resolve rather than silently becoming Locked —
/// that would make Guard enforce a value the user never enabled.
#[test]
fn every_fixture_parameter_migrates_to_the_lifecycle_its_name_claims() {
    let params = v0_params();
    let expected = [
        ("disabled", Ok(ParameterLifecycle::Disabled)),
        ("applied", Ok(ParameterLifecycle::Applied)),
        ("locked", Ok(ParameterLifecycle::Locked)),
        ("invalid", Err(())),
    ];

    for (id, want) in expected {
        let param = params.get(id).unwrap_or_else(|| panic!("fixture has {id}"));
        let (applied, locked) = flags(param);
        assert_eq!(
            ParameterLifecycle::from_legacy_flags(applied, locked).map_err(|_| ()),
            want,
            "fixture parameter {id} migrated to the wrong lifecycle"
        );
    }

    // 非法组合绝不能被解释成任何一种"继续看守"的状态。
    let (applied, locked) = flags(&params["invalid"]);
    let resolved = ParameterLifecycle::from_legacy_flags(applied, locked)
        .unwrap_or(ParameterLifecycle::Disabled);
    assert!(
        !resolved.is_enabled() && !resolved.is_locked(),
        "an unresolvable pair must fall back to a non-enforcing state"
    );
}
