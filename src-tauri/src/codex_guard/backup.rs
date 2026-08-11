//! Durable backups for Guard writes.
//!
//! A backup is scoped by the normalized relative target and transaction batch.  The
//! old second-based flat filename could overwrite a backup when two writes happened
//! in the same second (and also conflated `a/b` with `a_b`).  Backups are now created
//! with `create_new`, synced before returning, and retained per physical target.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::i18n::trf;

use super::{now_secs, AppPaths};

const RETENTION_LIMIT: usize = 20;
static NEXT_BACKUP_ID: AtomicU64 = AtomicU64::new(1);

/// Write a durable, collision-resistant backup for one transaction participant.
pub(crate) fn backup_before_write_for_batch(
    paths: &AppPaths,
    rel_file: &str,
    target: &Path,
    batch_id: &str,
) -> Result<(), String> {
    backup(paths, rel_file, target, batch_id)
}

fn backup(paths: &AppPaths, rel_file: &str, target: &Path, batch_id: &str) -> Result<(), String> {
    let Some(relative) = validate_relative_path(rel_file) else {
        return Err("backup target must be a normalized relative path".to_string());
    };
    if !target_exists_as_regular_file(target)? {
        return Ok(());
    }
    if batch_id.is_empty()
        || batch_id.contains(['/', '\\', '\0'])
        || batch_id == "."
        || batch_id == ".."
    {
        return Err("backup batch id is invalid".to_string());
    }

    let target_dir = paths
        .backup_root()
        .join("codex")
        .join(relative.parent().unwrap_or_else(|| Path::new("")));
    // Check the nearest already-existing ancestor before `create_dir_all`; otherwise
    // a pre-existing symlink in the path could redirect directory creation outside
    // the backup root before the later component check gets a chance to reject it.
    ensure_no_symlink_ancestors(&target_dir)?;
    fs::create_dir_all(&target_dir).map_err(|e| {
        trf(
            "Failed to create backup directory: {error}",
            &[("error", e.to_string())],
        )
    })?;
    ensure_no_symlink_components(paths.backup_root(), &target_dir)?;

    let file_name = relative
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "backup target filename is invalid".to_string())?;
    let stem = format!("{}.dashi-backup", file_name);
    let dest = create_unique_backup(&target_dir, &stem, batch_id, target)?;
    sync_directory_chain(paths.backup_root(), &target_dir)?;
    retain_latest(&target_dir, &stem)?;
    sync_directory_chain(paths.backup_root(), &target_dir)?;
    Ok(dest).map(|_| ())
}

fn target_exists_as_regular_file(target: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("backup target is not a regular file".to_string())
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(trf(
            "Backup failed: {error}",
            &[("error", error.to_string())],
        )),
    }
}

fn create_unique_backup(
    dir: &Path,
    stem: &str,
    batch_id: &str,
    source: &Path,
) -> Result<PathBuf, String> {
    let timestamp = now_secs();
    for attempt in 0..32u32 {
        let serial = NEXT_BACKUP_ID.fetch_add(1, Ordering::Relaxed);
        let suffix = if attempt == 0 {
            format!("{timestamp}-{serial}-{batch_id}")
        } else {
            format!("{timestamp}-{serial}-{batch_id}-{attempt}")
        };
        let dest = dir.join(format!("{stem}.{suffix}.bak"));
        let staging = dir.join(format!(".{stem}.{suffix}.tmp"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = match options.open(&staging) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(trf(
                    "Backup failed: {error}",
                    &[("error", error.to_string())],
                ))
            }
        };
        let result = (|| {
            let mut source_file = File::open(source)
                .map_err(|error| trf("Backup failed: {error}", &[("error", error.to_string())]))?;
            io::copy(&mut source_file, &mut file)
                .map_err(|error| trf("Backup failed: {error}", &[("error", error.to_string())]))?;
            file.flush().and_then(|_| file.sync_all()).map_err(|error| {
                trf(
                    "Backup sync failed: {error}",
                    &[("error", error.to_string())],
                )
            })
        })();
        if let Err(error) = result {
            drop(file);
            let _ = fs::remove_file(&staging);
            return Err(error);
        }
        drop(file);

        // The staging file is fully synced before publication.  It has a name
        // outside the retention pattern, so a crash during the copy cannot
        // leave a partial `.bak` that looks like a usable backup.
        if let Err(error) = fs::rename(&staging, &dest) {
            let _ = fs::remove_file(&staging);
            return Err(trf(
                "Backup publish failed: {error}",
                &[("error", error.to_string())],
            ));
        }
        return Ok(dest);
    }
    Err("Backup failed: unable to allocate a unique filename".to_string())
}

fn retain_latest(dir: &Path, stem: &str) -> Result<(), String> {
    let mut backups: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| {
            trf(
                "Failed to read backup directory: {error}",
                &[("error", e.to_string())],
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&format!("{stem}.")) && name.ends_with(".bak"))
        })
        .collect();
    backups.sort_by_key(|path| backup_order_key(path, stem));
    while backups.len() > RETENTION_LIMIT {
        let old = backups.remove(0);
        fs::remove_file(&old).map_err(|error| {
            trf(
                "Failed to prune backup: {error}",
                &[("error", error.to_string())],
            )
        })?;
    }
    Ok(())
}

fn backup_order_key(path: &Path, stem: &str) -> (u64, u64, u32, String) {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let body = name
        .strip_prefix(&format!("{stem}."))
        .and_then(|value| value.strip_suffix(".bak"))
        .unwrap_or_default();
    let parts = body.split('-').collect::<Vec<_>>();
    let timestamp = parts
        .first()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let serial = parts
        .get(1)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let attempt = parts
        .last()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|_| parts.len() > 3)
        .unwrap_or(0);
    (timestamp, serial, attempt, name.to_string())
}

fn validate_relative_path(value: &str) -> Option<&Path> {
    let path = Path::new(value);
    if value.is_empty() || path.is_absolute() {
        return None;
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => return None,
        }
    }
    Some(path)
}

fn ensure_no_symlink_ancestors(path: &Path) -> Result<(), String> {
    let mut current = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err("backup path contains a symlink or non-directory".to_string());
                }
                // The first existing ancestor is enough: all missing descendants
                // will be created below and checked component-by-component after
                // `create_dir_all`.  Do not walk into unrelated system ancestors
                // (some platforms expose those through symlinks).
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if !current.pop() {
                    return Err("backup path has no existing parent directory".to_string());
                }
            }
            Err(error) => {
                return Err(trf(
                    "Failed to inspect backup path: {error}",
                    &[("error", error.to_string())],
                ))
            }
        }
    }
}

fn ensure_no_symlink_components(root: &Path, target_dir: &Path) -> Result<(), String> {
    let root_metadata = fs::symlink_metadata(root).map_err(|error| {
        trf(
            "Failed to inspect backup root: {error}",
            &[("error", error.to_string())],
        )
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("backup root contains a symlink or non-directory".to_string());
    }
    let relative = target_dir
        .strip_prefix(root)
        .map_err(|_| "backup directory escaped backup root".to_string())?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err("backup directory component is invalid".to_string());
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            trf(
                "Failed to inspect backup directory: {error}",
                &[("error", error.to_string())],
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("backup directory contains a symlink or non-directory".to_string());
        }
    }
    Ok(())
}

fn sync_directory_chain(root: &Path, leaf: &Path) -> Result<(), String> {
    let mut current = leaf.to_path_buf();
    let mut directories = Vec::new();
    loop {
        if !current.starts_with(root) {
            return Err("backup directory escaped backup root".to_string());
        }
        directories.push(current.clone());
        if current == root {
            break;
        }
        if !current.pop() {
            return Err("backup directory has no backup root ancestor".to_string());
        }
    }
    for directory in directories {
        super::atomic_store::sync_directory(&directory)
            .map_err(|error| trf("Backup directory sync failed: {error}", &[("error", error)]))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::AppPaths;

    #[test]
    fn same_second_and_batch_still_create_distinct_backups() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("agents/default.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"one").unwrap();

        backup_before_write_for_batch(&paths, "agents/default.toml", &target, "batch-a").unwrap();
        backup_before_write_for_batch(&paths, "agents/default.toml", &target, "batch-a").unwrap();

        let files: Vec<_> = fs::read_dir(paths.backup_root().join("codex/agents"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(files.len(), 2);
        assert_eq!(fs::read(&files[0]).unwrap(), b"one");
        assert_eq!(fs::read(&files[1]).unwrap(), b"one");
        assert!(files.iter().all(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".bak"))
        }));
    }

    #[test]
    fn retention_is_per_target_and_keeps_exactly_twenty() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        for index in 0..25 {
            fs::write(&target, format!("{index}")).unwrap();
            backup_before_write_for_batch(
                &paths,
                "config.toml",
                &target,
                &format!("batch-{index}"),
            )
            .unwrap();
        }
        let files: Vec<_> = fs::read_dir(paths.backup_root().join("codex"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(files.len(), RETENTION_LIMIT);
        assert!(files
            .iter()
            .all(|path| path.extension().and_then(|ext| ext.to_str()) == Some("bak")));
    }

    #[test]
    fn failed_source_open_removes_partial_backup() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing.toml");

        let error = create_unique_backup(temp.path(), "config.dashi-backup", "batch-a", &missing)
            .unwrap_err();

        assert!(error.contains("Backup failed"));
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn backup_is_created_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"private").unwrap();

        backup_before_write_for_batch(&paths, "config.toml", &target, "batch-private").unwrap();

        let backup = fs::read_dir(paths.backup_root().join("codex"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn backup_rejects_a_symlinked_backup_root() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        fs::create_dir_all(paths.codex_root()).unwrap();
        let outside = temp.path().join("outside-backups");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, paths.backup_root()).unwrap();

        let target = paths.codex_file("config.toml");
        fs::write(&target, b"one").unwrap();
        let error =
            backup_before_write_for_batch(&paths, "config.toml", &target, "batch-a").unwrap_err();

        assert!(error.contains("symlink"));
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn backup_rejects_a_symlinked_backup_parent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        fs::create_dir_all(paths.backup_root()).unwrap();
        let outside = temp.path().join("outside-backups");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, paths.backup_root().join("codex")).unwrap();

        let target = paths.codex_file("config.toml");
        fs::write(&target, b"one").unwrap();
        let error =
            backup_before_write_for_batch(&paths, "config.toml", &target, "batch-a").unwrap_err();

        assert!(error.contains("symlink"));
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }
}
