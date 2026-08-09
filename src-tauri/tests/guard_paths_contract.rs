use std::path::{Path, PathBuf};

fn rust_sources(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("read Rust source directory") {
        let path = entry.expect("read Rust source entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn home_environment_is_read_only_in_the_composition_root() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = manifest.join("src");
    let main_rs = source_root.join("main.rs");
    let mut sources = Vec::new();
    rust_sources(&source_root, &mut sources);

    for needle in ["std::env::var(\"HOME\")", "std::env::var(\"USERPROFILE\")"] {
        let mut matches = Vec::new();
        for source in &sources {
            let content = std::fs::read_to_string(source).expect("read Rust source");
            for (index, line) in content.lines().enumerate() {
                if line.contains(needle) {
                    matches.push((source.clone(), index + 1));
                }
            }
        }

        assert_eq!(
            matches,
            vec![(
                main_rs.clone(),
                matches.first().map_or(0, |(_, line)| *line)
            )],
            "{needle} must appear exactly once and only in src/main.rs: {matches:?}",
        );
    }
}
