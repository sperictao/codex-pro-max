//! Capability discovery for the configured Codex Desktop installation.
//!
//! The capability boundary intentionally has no shell or PATH fallback.  A
//! configured app path is resolved to the bundled `codex` executable, and the
//! executable is asked for the two machine-readable reports that define the
//! model contract.  The command runner is kept behind a small trait so parser
//! and timeout behavior can be tested with deterministic fixtures.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::roles::{CapabilitySnapshot, ModelCapability, RoleError};

pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;

const DOCTOR_ARGS: &[&str] = &["doctor", "--json"];
const MODELS_ARGS: &[&str] = &["debug", "models"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: ExitStatus,
}

pub(crate) trait CapabilityCommandRunner {
    fn run(&self, executable: &Path, args: &[&str]) -> Result<CommandOutput, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProcessCapabilityCommandRunner;

impl CapabilityCommandRunner for ProcessCapabilityCommandRunner {
    fn run(&self, executable: &Path, args: &[&str]) -> Result<CommandOutput, String> {
        run_bounded_command(executable, args)
    }
}

/// Resolve only inside the configured installation.  We deliberately do not
/// invoke `which`, inspect PATH, or accept an arbitrary third-party launcher.
pub(crate) fn resolve_codex_executable(configured_path: &str) -> Result<PathBuf, String> {
    let configured_path = configured_path.trim();
    if configured_path.is_empty() {
        return Err("capability_executable_not_configured".to_string());
    }
    if configured_path.starts_with("msix:") {
        #[cfg(windows)]
        {
            return resolve_windows_store_alias(&configured_path[5..]);
        }
        #[cfg(not(windows))]
        {
            // A Store app is an activation identity, not a filesystem executable.
            // The packaged binary cannot be safely resolved without the app's
            // installation metadata, so fail closed instead of falling back to PATH.
            return Err("capability_packaged_identity_unresolved".to_string());
        }
    }

    let configured = PathBuf::from(configured_path);
    let mut candidates = Vec::new();
    if configured.is_file() {
        candidates.push(configured.clone());
    }

    let looks_like_bundle = configured
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".app"));
    let install_root = if configured.is_dir() || looks_like_bundle {
        configured.clone()
    } else {
        configured
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };

    // macOS app bundles and the equivalent packaged layouts used by Linux and
    // Windows builds.  The configured path remains the trust boundary.
    for relative in [
        "Contents/Resources/codex",
        "Contents/Resources/bin/codex",
        "Contents/Resources/codex.exe",
        "Contents/Resources/bin/codex.exe",
        "Contents/Resources/app.asar.unpacked/codex",
        "Contents/Resources/app.asar.unpacked/bin/codex",
        "resources/codex",
        "resources/bin/codex",
        "resources/codex.exe",
        "resources/bin/codex.exe",
        "bin/codex",
        "bin/codex.exe",
        "codex",
        "codex.exe",
    ] {
        candidates.push(install_root.join(relative));
    }

    // If the configured path points at a bundled launcher binary, its sibling
    // resources are common in unpacked Linux installations.
    if let Some(parent) = Path::new(configured_path).parent() {
        candidates.push(parent.join("../lib/codex"));
        candidates.push(parent.join("../lib/codex.exe"));
    }

    candidates
        .into_iter()
        .find(|path| is_executable_file(path))
        .ok_or_else(|| "capability_executable_not_found".to_string())
}

#[cfg(windows)]
fn resolve_windows_store_alias(amid: &str) -> Result<PathBuf, String> {
    let package_family = amid.strip_suffix("!App").unwrap_or(amid);
    let known_family = [
        "OpenAI.Codex_",
        "OpenAI.CodexBeta_",
        "OpenAI.ChatGPT-Desktop_",
    ]
    .iter()
    .any(|prefix| package_family.starts_with(prefix));
    if !known_family {
        return Err("capability_packaged_identity_unresolved".to_string());
    }
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "capability_packaged_identity_unresolved".to_string())?;
    let aliases_root = local_app_data.join("Microsoft").join("WindowsApps");
    ["Codex.exe", "ChatGPT.exe", "codex.exe", "chatgpt.exe"]
        .into_iter()
        .map(|name| aliases_root.join(name))
        .find(|path| is_executable_file(path))
        .ok_or_else(|| "capability_packaged_identity_unresolved".to_string())
}

pub(crate) fn current_capability(
    configured_path: &str,
    now_ms: u64,
) -> Result<CapabilitySnapshot, String> {
    let executable = resolve_codex_executable(configured_path)?;
    let runner = ProcessCapabilityCommandRunner;
    probe_with_runner(&executable, &runner, now_ms)
}

pub(crate) fn probe_with_runner<R: CapabilityCommandRunner + ?Sized>(
    executable: &Path,
    runner: &R,
    fetched_at_ms: u64,
) -> Result<CapabilitySnapshot, String> {
    if !is_executable_file(executable) {
        return Err("capability_executable_not_found".to_string());
    }
    let identity = executable_identity(executable)?;
    let doctor = runner.run(executable, DOCTOR_ARGS)?;
    ensure_success(&doctor)?;
    let models = runner.run(executable, MODELS_ARGS)?;
    ensure_success(&models)?;
    parse_reports(&doctor.stdout, &models.stdout, identity, fetched_at_ms)
}

fn ensure_success(output: &CommandOutput) -> Result<(), String> {
    if output.status.success() {
        Ok(())
    } else {
        Err("capability_command_failed".to_string())
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn executable_identity(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "capability_identity_unavailable".to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "capability_identity_unavailable".to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn run_bounded_command(executable: &Path, args: &[&str]) -> Result<CommandOutput, String> {
    run_bounded_command_with_limits(executable, args, COMMAND_TIMEOUT, MAX_OUTPUT_BYTES)
}

fn run_bounded_command_with_limits(
    executable: &Path,
    args: &[&str],
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<CommandOutput, String> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "capability_command_spawn_failed".to_string())?;
    collect_bounded_output(&mut child, timeout, max_output_bytes)
}

fn collect_bounded_output(
    child: &mut Child,
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<CommandOutput, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "capability_command_spawn_failed".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "capability_command_spawn_failed".to_string())?;
    let output_too_large = Arc::new(AtomicBool::new(false));
    let stdout_too_large = Arc::clone(&output_too_large);
    let stderr_too_large = Arc::clone(&output_too_large);
    let stdout_thread =
        thread::spawn(move || read_bounded(stdout, stdout_too_large, max_output_bytes));
    let stderr_thread =
        thread::spawn(move || read_bounded(stderr, stderr_too_large, max_output_bytes));

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if output_too_large.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return Err("capability_command_output_too_large".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err("capability_command_timeout".to_string());
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err("capability_command_wait_failed".to_string());
            }
        }
    };

    let stdout = stdout_thread
        .join()
        .map_err(|_| "capability_command_read_failed".to_string())??;
    let stderr = stderr_thread
        .join()
        .map_err(|_| "capability_command_read_failed".to_string())??;
    Ok(CommandOutput {
        stdout,
        stderr,
        status,
    })
}

fn read_bounded<R: Read>(
    mut reader: R,
    too_large: Arc<AtomicBool>,
    max_output_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| "capability_command_read_failed".to_string())?;
        if read == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(read) > max_output_bytes {
            too_large.store(true, Ordering::Release);
            return Err("capability_command_output_too_large".to_string());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
}

pub(crate) fn parse_reports(
    doctor_bytes: &[u8],
    models_bytes: &[u8],
    executable_identity: String,
    fetched_at_ms: u64,
) -> Result<CapabilitySnapshot, String> {
    let doctor = parse_json(doctor_bytes)?;
    let models = parse_json(models_bytes)?;
    // The doctor report is intentionally parsed even when it carries no model
    // data: this verifies that config diagnostics are machine-readable and
    // prevents accidentally accepting a human-oriented fallback output.
    validate_doctor_shape(&doctor)?;
    let model_values =
        find_models(&models).ok_or_else(|| "capability_models_missing".to_string())?;
    let mut capabilities = Vec::with_capacity(model_values.len());
    for model in model_values {
        capabilities.push(parse_model(model)?);
    }
    if capabilities.is_empty() {
        return Err("capability_models_missing".to_string());
    }
    CapabilitySnapshot::new(capabilities, executable_identity, fetched_at_ms)
        .map_err(role_error_string)
}

fn parse_json(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(bytes).map_err(|_| "capability_invalid_json".to_string())
}

fn validate_doctor_shape(value: &Value) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Err("capability_invalid_doctor_report".to_string());
    };
    for key in ["config_loaded", "configLoaded"] {
        if let Some(value) = object.get(key) {
            if !value.is_boolean() {
                return Err("capability_invalid_doctor_report".to_string());
            }
            if value.as_bool() == Some(false) {
                return Err("capability_config_unavailable".to_string());
            }
        }
    }
    if let Some(config) = object.get("config") {
        if let Some(config_object) = config.as_object() {
            for key in ["loaded", "config_loaded", "configLoaded"] {
                if let Some(value) = config_object.get(key) {
                    if !value.is_boolean() {
                        return Err("capability_invalid_doctor_report".to_string());
                    }
                    if value.as_bool() == Some(false) {
                        return Err("capability_config_unavailable".to_string());
                    }
                }
            }
        } else if !config.is_null() {
            return Err("capability_invalid_doctor_report".to_string());
        }
    }
    for key in ["role_warnings", "roleWarnings"] {
        if let Some(value) = object.get(key) {
            if !value.is_array() {
                return Err("capability_invalid_doctor_report".to_string());
            }
        }
    }
    Ok(())
}

fn find_models(value: &Value) -> Option<&[Value]> {
    if let Some(models) = value.as_array() {
        return Some(models);
    }
    if let Some(models) = value.get("models").and_then(Value::as_array) {
        return Some(models);
    }
    value.as_object()?.values().find_map(find_models)
}

fn parse_model(value: &Value) -> Result<ModelCapability, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "capability_model_invalid".to_string())?;
    let slug = object
        .get("slug")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| "capability_model_invalid".to_string())?;
    let multi_agent_version = object
        .get("multi_agent_version")
        .or_else(|| object.get("multiAgentVersion"))
        .and_then(Value::as_str)
        .ok_or_else(|| "capability_model_invalid".to_string())?;
    let levels = object
        .get("supported_reasoning_levels")
        .or_else(|| object.get("supportedReasoningLevels"))
        .and_then(Value::as_array)
        .ok_or_else(|| "capability_model_invalid".to_string())?;
    let mut efforts = Vec::with_capacity(levels.len());
    for level in levels {
        let effort = level
            .as_str()
            .or_else(|| level.get("effort").and_then(Value::as_str))
            .ok_or_else(|| "capability_model_invalid".to_string())?;
        efforts.push(effort.to_string());
    }
    ModelCapability::new(slug, multi_agent_version, efforts).map_err(role_error_string)
}

fn role_error_string(error: RoleError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::process::ExitStatus;

    struct FixtureRunner {
        doctor: Vec<u8>,
        models: Vec<u8>,
        status: ExitStatus,
    }

    impl CapabilityCommandRunner for FixtureRunner {
        fn run(&self, _executable: &Path, args: &[&str]) -> Result<CommandOutput, String> {
            let stdout = if args == DOCTOR_ARGS {
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

    fn fixture_executable() -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        #[cfg(unix)]
        {
            let mut permissions = file.as_file().metadata().unwrap().permissions();
            permissions.set_mode(0o755);
            file.as_file().set_permissions(permissions).unwrap();
        }
        file
    }

    fn ok_runner() -> FixtureRunner {
        FixtureRunner {
            doctor: br#"{"config_loaded":true,"role_warnings":[]}"#.to_vec(),
            models: br#"{"models":[{"slug":"gpt-5","multi_agent_version":"v2","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]}]}"#.to_vec(),
            status: success_status(),
        }
    }

    #[test]
    fn fixture_probe_parses_only_machine_fields() {
        let executable = fixture_executable();
        let snapshot = probe_with_runner(executable.path(), &ok_runner(), 123).unwrap();
        assert_eq!(snapshot.executable_identity.len(), 64);
        assert_eq!(snapshot.available_models(), vec!["gpt-5"]);
        assert!(snapshot.supports("gpt-5", "high"));
    }

    #[test]
    fn malformed_or_missing_models_are_rejected() {
        let executable = fixture_executable();
        let runner = FixtureRunner {
            doctor: br#"{}"#.to_vec(),
            models: br#"{"models":[]}"#.to_vec(),
            status: success_status(),
        };
        assert_eq!(
            probe_with_runner(executable.path(), &runner, 123).unwrap_err(),
            "capability_models_missing"
        );
    }

    #[test]
    fn configured_path_resolves_bundled_binary_without_path_lookup() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("ChatGPT.app");
        let executable = app.join("Contents/Resources/codex");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, b"fixture").unwrap();
        #[cfg(unix)]
        {
            let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&executable, permissions).unwrap();
        }
        assert_eq!(
            resolve_codex_executable(app.to_str().unwrap()),
            Ok(executable)
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_runner_enforces_timeout_without_shell_fallback() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("slow-codex");
        std::fs::write(&executable, b"#!/bin/sh\nsleep 1\n").unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();
        assert_eq!(
            run_bounded_command_with_limits(
                &executable,
                &[],
                Duration::from_millis(30),
                MAX_OUTPUT_BYTES,
            )
            .unwrap_err(),
            "capability_command_timeout"
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_runner_limits_each_output_stream() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("loud-codex");
        std::fs::write(
            &executable,
            b"#!/bin/sh\nprintf '0123456789abcdef0123456789abcdef'\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();
        assert_eq!(
            run_bounded_command_with_limits(&executable, &[], Duration::from_secs(1), 16)
                .unwrap_err(),
            "capability_command_output_too_large"
        );
    }

    #[cfg(unix)]
    fn success_status() -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }

    #[cfg(windows)]
    fn success_status() -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }
}
