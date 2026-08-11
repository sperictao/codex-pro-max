#![allow(dead_code)]

#[path = "../src/codex_guard/roles.rs"]
mod roles;

use roles::{
    plan_default_role_migration, resolve_default_role_migration, RoleLifecycle,
    RoleMigrationChoice, RoleMigrationPendingReason, RoleMigrationSource,
};

fn role_toml(name: &str) -> String {
    format!(
        "name = \"{name}\"\ndescription = \"A role\"\nmodel = \"gpt-5\"\nmodel_reasoning_effort = \"low\"\ndeveloper_instructions = \"Do the work.\"\n"
    )
}

#[test]
fn equal_live_and_stored_sources_auto_migrate_without_overwrite() {
    let stored = role_toml("Stored");
    let plan = plan_default_role_migration(
        Some(&stored),
        Some(&stored),
        RoleLifecycle::Locked,
        &role_toml("Safe"),
    )
    .expect("migration plan");
    assert_eq!(
        plan,
        roles::RoleMigrationPlan::Auto {
            source: RoleMigrationSource::Stored,
            lifecycle: RoleLifecycle::Locked,
        }
    );
}

#[test]
fn conflicting_sources_require_an_explicit_choice() {
    let stored = role_toml("Stored");
    let live = role_toml("Live");
    let plan = plan_default_role_migration(
        Some(&stored),
        Some(&live),
        RoleLifecycle::Locked,
        &role_toml("Safe"),
    )
    .expect("migration plan");
    match plan {
        roles::RoleMigrationPlan::Pending { reason, choices } => {
            assert_eq!(reason, RoleMigrationPendingReason::Conflict);
            assert_eq!(
                choices,
                vec![
                    RoleMigrationChoice::AdoptLive,
                    RoleMigrationChoice::AdoptStored,
                    RoleMigrationChoice::UseSafeDefault,
                ]
            );
        }
        roles::RoleMigrationPlan::Auto { .. } => panic!("conflict must not auto-migrate"),
    }
}

#[test]
fn resolving_a_different_source_disables_a_locked_role() {
    let stored = role_toml("Stored");
    let live = role_toml("Live");
    let result = resolve_default_role_migration(
        RoleMigrationChoice::AdoptStored,
        Some(&stored),
        Some(&live),
        &role_toml("Safe"),
        RoleLifecycle::Locked,
        99,
    )
    .expect("migration result");
    assert_eq!(result.source, RoleMigrationSource::Stored);
    assert_eq!(result.lifecycle, RoleLifecycle::Disabled);
    assert_eq!(result.record.id.as_str(), "default");
    assert_eq!(result.record.policy_updated_at_ms, 99);
}
