#[path = "../src/codex_guard/atomic_store.rs"]
mod atomic_store;

use atomic_store::{AtomicFileWriter, PlatformAtomicFileWriter};

#[test]
fn replacement_is_atomic_from_the_callers_point_of_view() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let target = temp.path().join("nested").join("config.toml");

    PlatformAtomicFileWriter
        .replace(&target, b"first")
        .expect("initial replacement");
    assert_eq!(std::fs::read(&target).unwrap(), b"first");

    PlatformAtomicFileWriter
        .replace(&target, b"second")
        .expect("second replacement");
    assert_eq!(std::fs::read(&target).unwrap(), b"second");

    let temporary_files = std::fs::read_dir(target.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".dashi-"))
        .collect::<Vec<_>>();
    assert!(
        temporary_files.is_empty(),
        "temporary file leaked: {temporary_files:?}"
    );
}

#[test]
fn replacement_preserves_existing_permissions_and_rejects_readonly_targets() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let target = temp.path().join("config.json");
    std::fs::write(&target, b"old").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        PlatformAtomicFileWriter.replace(&target, b"new").unwrap();
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let original_permissions = std::fs::metadata(&target).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&target, readonly).unwrap();
    let error = PlatformAtomicFileWriter
        .replace(&target, b"blocked")
        .unwrap_err();
    assert_eq!(error, "atomic target is read-only");
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    std::fs::set_permissions(&target, original_permissions).unwrap();
}

#[cfg(unix)]
#[test]
fn remove_regular_file_refuses_socket_targets() {
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::net::UnixListener;

    let temp = tempfile::tempdir().expect("temporary directory");
    let target = temp.path().join("target.socket");
    let _listener = UnixListener::bind(&target).unwrap();

    let error = atomic_store::remove_regular_file(&target).unwrap_err();

    assert!(error.contains("not a regular file"));
    assert!(std::fs::symlink_metadata(&target)
        .unwrap()
        .file_type()
        .is_socket());
}
