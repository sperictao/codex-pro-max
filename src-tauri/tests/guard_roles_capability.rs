#![allow(dead_code)]

#[path = "../src/codex_guard/capability.rs"]
mod capability;
#[path = "../src/codex_guard/roles.rs"]
mod roles;

use capability::{probe_with_runner, CapabilityCommandRunner, CommandOutput};
use roles::{
    parse_role_toml, validate_role_capability_values, CapabilitySnapshot, ModelCapability,
    RoleErrorCode,
};
use std::path::Path;
use std::process::ExitStatus;

struct FixtureRunner {
    doctor: Vec<u8>,
    models: Vec<u8>,
    status: ExitStatus,
}

impl CapabilityCommandRunner for FixtureRunner {
    fn run(&self, _executable: &Path, args: &[&str]) -> Result<CommandOutput, String> {
        let stdout = if args == ["doctor", "--json"] {
            self.doctor.clone()
        } else {
            self.models.clone()
        };
        Ok(CommandOutput {
            stdout,
            stderr: Vec::new(),
            status: self.status,
        })
    }
}

fn success_status() -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }
}

fn role_toml(model: &str, effort: &str) -> String {
    format!(
        "name = \"Reviewer\"\ndescription = \"Review changes\"\nmodel = \"{model}\"\nmodel_reasoning_effort = \"{effort}\"\ndeveloper_instructions = \"Review the change.\"\n"
    )
}

#[test]
fn capability_probe_and_role_validation_share_the_same_v2_snapshot() {
    let temp = tempfile::tempdir().expect("temp executable directory");
    let executable = temp.path().join("codex");
    std::fs::write(&executable, b"fixture").expect("write fixture executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();
    }
    let runner = FixtureRunner {
        doctor: br#"{"config_loaded":true,"role_warnings":[]}"#.to_vec(),
        models: br#"{"models":[{"slug":"gpt-5","multi_agent_version":"v2","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]}]}"#.to_vec(),
        status: success_status(),
    };

    let snapshot = probe_with_runner(&executable, &runner, 123).expect("capability snapshot");
    assert_eq!(snapshot.available_models(), vec!["gpt-5"]);
    assert!(snapshot.supports("gpt-5", "high"));

    let parsed = parse_role_toml(&role_toml("gpt-5", "high")).expect("valid role");
    validate_role_capability_values(&parsed.view.model, &parsed.view.effort, &snapshot)
        .expect("role capability is supported");
}

#[test]
fn capability_probe_rejects_missing_models_and_v1_roles() {
    let temp = tempfile::tempdir().expect("temp executable directory");
    let executable = temp.path().join("codex");
    std::fs::write(&executable, b"fixture").expect("write fixture executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();
    }
    let runner = FixtureRunner {
        doctor: br#"{"config_loaded":true,"role_warnings":[]}"#.to_vec(),
        models: br#"{"models":[]}"#.to_vec(),
        status: success_status(),
    };
    assert_eq!(
        probe_with_runner(&executable, &runner, 123).unwrap_err(),
        "capability_models_missing"
    );

    let snapshot = CapabilitySnapshot::new(
        vec![ModelCapability::new("gpt-legacy", "v1", vec!["low".into()]).unwrap()],
        "fixture",
        123,
    )
    .unwrap();
    let error = validate_role_capability_values("gpt-legacy", "low", &snapshot).unwrap_err();
    assert_eq!(error.code, RoleErrorCode::UnsupportedMultiAgentVersion);
}
