//! Behavioural contract for the Guard command registry.
//!
//! A Rust test suite can be entirely green while the frontend's `invoke` fails, because
//! nothing in the crate forces the specta `collect_commands!` list, the Tauri
//! `invoke_handler` list and the generated TypeScript to agree. This target closes that
//! gap by running the shipped contract generator — the same entrypoint CI uses — and
//! asserting against the emitted `TAURI_INVOKE` names, which is literally the string the
//! frontend sends at runtime.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct CommandFixture {
    #[serde(rename = "schemaVersion")]
    schema_version: u16,
    commands: Vec<String>,
}

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixture() -> CommandFixture {
    serde_json::from_slice(
        &std::fs::read(manifest().join("src/codex_guard/fixtures/contracts/command-registry.json"))
            .expect("read Guard command registry fixture"),
    )
    .expect("parse Guard command registry fixture")
}

/// Runs the production `--generate-guard-contracts` entrypoint and returns the command
/// names the generated bindings will actually invoke, in emission order.
fn generated_invoke_names() -> (Vec<String>, PathBuf) {
    let out = std::env::temp_dir().join(format!(
        "dashi-guard-registry-{}-{}.ts",
        std::process::id(),
        line!()
    ));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_dashi-taskboard-launcher"))
        .arg("--generate-guard-contracts")
        .arg(&out)
        .status()
        .expect("run the contract generator");
    assert!(status.success(), "contract generation must succeed");

    let generated = std::fs::read_to_string(&out).expect("read generated bindings");
    let mut names = Vec::new();
    for (_, rest) in generated
        .match_indices("TAURI_INVOKE(\"")
        .map(|(index, matched)| (index, &generated[index + matched.len()..]))
    {
        let name = rest
            .split('"')
            .next()
            .expect("invoke name is quoted")
            .to_string();
        names.push(name);
    }
    (names, out)
}

/// The generated bindings must invoke exactly the commands the fixture pins, in order.
/// Renaming a command, dropping it from `invoke_handler`, or adding one without updating
/// the contract all fail here.
#[test]
fn generated_bindings_invoke_exactly_the_contracted_commands() {
    let fixture = fixture();
    assert_eq!(fixture.schema_version, 1);

    let (invoked, out) = generated_invoke_names();
    let _ = std::fs::remove_file(&out);

    assert_eq!(
        invoked, fixture.commands,
        "the generated bindings and the pinned command contract disagree"
    );
}

/// `collect_commands!` feeds the bindings, but Tauri only accepts an `invoke` for a command
/// listed in `invoke_handler`. The two lists are written separately, so a command can be
/// generated for the frontend yet rejected at runtime. Assert every generated command is
/// also handed to the Tauri handler.
#[test]
fn every_generated_command_is_also_registered_with_the_tauri_handler() {
    let (invoked, out) = generated_invoke_names();
    let _ = std::fs::remove_file(&out);

    let main = std::fs::read_to_string(manifest().join("src/main.rs")).expect("read main.rs");
    let handler = main
        .split_once("generate_handler![")
        .map(|(_, rest)| {
            rest.split_once(']')
                .expect("invoke_handler list must be closed")
                .0
        })
        .expect("main.rs must register an invoke handler");

    for command in &invoked {
        assert!(
            handler.contains(&format!("codex_guard::{command},")),
            "command `{command}` is generated for the frontend but never registered with Tauri"
        );
    }
}

/// Command names cross a process boundary as raw strings. Keeping them to a conservative
/// character set means a rename cannot smuggle in quoting or separator characters.
#[test]
fn contracted_command_names_are_stable_identifiers() {
    for command in fixture().commands {
        assert!(
            command.starts_with("guard_"),
            "`{command}` must stay inside the guard namespace"
        );
        assert!(
            command
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
            "`{command}` must be a lowercase snake_case identifier"
        );
    }
}
