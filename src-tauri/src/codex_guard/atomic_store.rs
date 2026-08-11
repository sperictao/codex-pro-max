use std::fs::File;
#[cfg(not(unix))]
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) trait AtomicFileWriter: Send + Sync {
    fn replace(&self, target: &Path, bytes: &[u8]) -> Result<(), String>;
}

/// Flush a directory entry update after creating, replacing, or removing a file.
///
/// Unix filesystems expose directory handles directly. Windows requires a directory
/// handle opened with `FILE_FLAG_BACKUP_SEMANTICS`; `FlushFileBuffers` then provides
/// the corresponding durable-boundary guarantee. Other platforms retain the best
/// file-level guarantee available to their standard library.
#[allow(dead_code)]
pub(crate) fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("failed to sync atomic target directory: {error}"))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FlushFileBuffers, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_WRITE_THROUGH,
            FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING,
        };

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_WRITE_THROUGH,
                None,
            )
        }
        .map_err(|error| format!("failed to open atomic target directory: {error}"))?;
        let result = unsafe { FlushFileBuffers(handle) }
            .map_err(|error| format!("failed to sync atomic target directory: {error}"));
        let _ = unsafe { CloseHandle(handle) };
        result
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = path;
        Ok(())
    }
}

/// Remove one regular file without following a parent-directory symlink, then flush
/// the containing directory entry. The Unix implementation uses an open directory
/// handle so a check-then-remove race cannot redirect the operation.
#[allow(dead_code)]
pub(crate) fn remove_regular_file(target: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use rustix::fs::{unlinkat, AtFlags};
        use rustix::io::Errno;

        let parent = target
            .parent()
            .ok_or_else(|| "atomic target has no parent directory".to_string())?;
        let target_name = target
            .file_name()
            .ok_or_else(|| "atomic target filename is missing".to_string())?;
        let parent_fd = open_directory_without_symlinks(parent)?;
        match std::fs::symlink_metadata(target) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err("atomic target is not a regular file".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("failed to inspect atomic target: {error}")),
        }
        match unlinkat(&parent_fd, target_name, AtFlags::empty()) {
            Ok(()) => sync_directory(parent),
            Err(error) if error == Errno::NOENT => Ok(()),
            Err(error) => Err(format!("failed to remove atomic target: {error}")),
        }
    }
    #[cfg(not(unix))]
    {
        match std::fs::symlink_metadata(target) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err("atomic target is not a regular file".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("failed to inspect atomic target: {error}")),
        }
        std::fs::remove_file(target)
            .map_err(|error| format!("failed to remove atomic target: {error}"))?;
        sync_directory(
            target
                .parent()
                .ok_or_else(|| "atomic target has no parent directory".to_string())?,
        )
    }
}

#[derive(Debug, Default)]
pub(crate) struct PlatformAtomicFileWriter;

impl AtomicFileWriter for PlatformAtomicFileWriter {
    fn replace(&self, target: &Path, bytes: &[u8]) -> Result<(), String> {
        let parent = target
            .parent()
            .ok_or_else(|| "atomic target has no parent directory".to_string())?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create atomic target directory: {error}"))?;

        let target_permissions = match std::fs::metadata(target) {
            Ok(metadata) => {
                let permissions = metadata.permissions();
                if permissions.readonly() {
                    return Err("atomic target is read-only".to_string());
                }
                Some(permissions)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("failed to inspect atomic target: {error}")),
        };

        #[cfg(unix)]
        {
            write_and_replace_unix(parent, target, bytes, target_permissions)
        }
        #[cfg(not(unix))]
        {
            write_and_replace_path(target, bytes, target_permissions)
        }
    }
}

#[cfg(not(unix))]
fn write_and_replace_path(
    target: &Path,
    bytes: &[u8],
    target_permissions: Option<std::fs::Permissions>,
) -> Result<(), String> {
    let temp = temp_path(target)?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| format!("failed to create atomic temp file: {error}"))?;
        write_temp_file(&mut file, bytes, target_permissions)?;
        drop(file);
        replace_file(&temp, target)
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.and_then(|()| sync_parent(target))
}

#[cfg(unix)]
fn write_and_replace_unix(
    parent: &Path,
    target: &Path,
    bytes: &[u8],
    target_permissions: Option<std::fs::Permissions>,
) -> Result<(), String> {
    use rustix::fs::{fsync, openat, renameat, unlinkat, AtFlags, Mode, OFlags};

    let parent_fd = open_directory_without_symlinks(parent)?;
    let target_name = target
        .file_name()
        .ok_or_else(|| "atomic target filename is missing".to_string())?;
    let temp_name = temp_path(target)?
        .file_name()
        .ok_or_else(|| "atomic temp filename is missing".to_string())?
        .to_os_string();
    let temp_fd = openat(
        &parent_fd,
        &temp_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| format!("failed to create atomic temp file: {error}"))?;
    let mut file = File::from(temp_fd);
    if let Err(error) = write_temp_file(&mut file, bytes, target_permissions) {
        let _ = unlinkat(&parent_fd, &temp_name, AtFlags::empty());
        return Err(error);
    }
    drop(file);
    if let Err(error) = renameat(&parent_fd, &temp_name, &parent_fd, target_name) {
        let _ = unlinkat(&parent_fd, &temp_name, AtFlags::empty());
        return Err(format!("failed to replace atomic target: {error}"));
    }
    fsync(&parent_fd)
        .map_err(|error| format!("failed to sync atomic target directory: {error}"))?;
    Ok(())
}

fn write_temp_file(
    file: &mut File,
    bytes: &[u8],
    target_permissions: Option<std::fs::Permissions>,
) -> Result<(), String> {
    file.write_all(bytes)
        .map_err(|error| format!("failed to write atomic temp file: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush atomic temp file: {error}"))?;
    if let Some(permissions) = target_permissions {
        file.set_permissions(permissions)
            .map_err(|error| format!("failed to preserve atomic target permissions: {error}"))?;
    }
    file.sync_all()
        .map_err(|error| format!("failed to sync atomic temp file: {error}"))
}

#[cfg(unix)]
fn open_directory_without_symlinks(path: &Path) -> Result<rustix::fd::OwnedFd, String> {
    use rustix::fs::{openat, Mode, OFlags, CWD};
    use std::path::Component;

    if !path.is_absolute() {
        return Err("atomic target parent must be absolute".to_string());
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize atomic target directory: {error}"))?;
    reject_user_symlink_components(path, &canonical)?;
    let mut directory = openat(
        CWD,
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| format!("failed to open atomic target root: {error}"))?;
    for component in canonical.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        directory = openat(
            &directory,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|error| format!("failed to open atomic target directory: {error}"))?;
    }
    Ok(directory)
}

#[cfg(unix)]
fn reject_user_symlink_components(path: &Path, canonical: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::RootDir => current.push("/"),
            std::path::Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            std::path::Component::Normal(name) => {
                current.push(name);
                let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
                    format!("failed to inspect atomic target directory: {error}")
                })?;
                if metadata.file_type().is_symlink() && !is_system_alias(&current, canonical) {
                    return Err("atomic target directory contains a symlink".to_string());
                }
            }
            std::path::Component::CurDir | std::path::Component::ParentDir => {}
        }
    }
    Ok(())
}

#[cfg(all(unix, target_os = "macos"))]
fn is_system_alias(path: &Path, canonical: &Path) -> bool {
    path == Path::new("/var") && canonical.starts_with("/private/var")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn is_system_alias(_path: &Path, _canonical: &Path) -> bool {
    false
}

fn temp_path(target: &Path) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| "atomic target has no parent directory".to_string())?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "atomic target filename is not valid UTF-8".to_string())?;
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(".{name}.dashi-{}-{id}.tmp", std::process::id())))
}

#[cfg(not(unix))]
fn sync_parent(target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "atomic target has no parent directory".to_string())?;
    sync_directory(parent)
}

#[cfg(all(not(unix), not(windows)))]
fn replace_file(temp: &Path, target: &Path) -> Result<(), String> {
    std::fs::rename(temp, target)
        .map_err(|error| format!("failed to replace atomic target: {error}"))
}

#[cfg(windows)]
fn replace_file(temp: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temp_wide: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(temp_wide.as_ptr()),
            PCWSTR(target_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| format!("failed to replace atomic target: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_writer_refuses_to_replace_readonly_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("config.json");
        std::fs::write(&target, b"old").unwrap();
        let original_permissions = std::fs::metadata(&target).unwrap().permissions();
        let mut readonly_permissions = original_permissions.clone();
        readonly_permissions.set_readonly(true);
        std::fs::set_permissions(&target, readonly_permissions).unwrap();

        let result = PlatformAtomicFileWriter.replace(&target, b"new");

        assert_eq!(result.unwrap_err(), "atomic target is read-only");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        std::fs::set_permissions(&target, original_permissions).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn platform_writer_preserves_existing_unix_mode() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("config.json");
        std::fs::write(&target, b"old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();

        PlatformAtomicFileWriter.replace(&target, b"new").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_failure_removes_the_atomic_temp_file() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        std::fs::create_dir(&target).unwrap();

        let error = PlatformAtomicFileWriter
            .replace(&target, b"new")
            .unwrap_err();

        assert!(error.contains("failed to replace atomic target"));
        assert_eq!(
            std::fs::read_dir(temp.path())
                .unwrap()
                .filter_map(Result::ok)
                .count(),
            1,
            "the pre-existing target directory is the only entry"
        );
    }
}
