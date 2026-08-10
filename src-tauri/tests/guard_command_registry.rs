use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct CommandFixture {
    #[serde(rename = "schemaVersion")]
    schema_version: u16,
    commands: Vec<String>,
}

#[test]
fn registered_guard_commands_match_contract_fixture() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture: CommandFixture = serde_json::from_slice(
        &std::fs::read(manifest.join("src/codex_guard/fixtures/contracts/command-registry.json"))
            .expect("read Guard command registry fixture"),
    )
    .expect("parse Guard command registry fixture");
    assert_eq!(fixture.schema_version, 1);

    let main = std::fs::read_to_string(manifest.join("src/main.rs")).expect("read main.rs");
    let registered = main
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("codex_guard::")
                .and_then(|line| line.strip_suffix(','))
                .map(str::to_string)
        })
        .collect::<Vec<_>>();

    assert_eq!(registered, fixture.commands);
}
